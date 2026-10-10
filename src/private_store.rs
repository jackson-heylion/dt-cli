//! Atomic, user-private local files, including credentials and recovery metadata.
use crate::output::{Failure, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

const MAX_METADATA: u64 = 16 * 1024;
#[cfg(windows)]
mod windows;

pub(crate) fn unavailable() -> Failure {
    Failure::new(
        "LOCAL_STATE_UNAVAILABLE",
        1,
        "本地账号选择或恢复记录不可用；未继续执行。",
    )
}

pub(crate) fn diagnostic(
    stage: &'static str,
    reason: &'static str,
    os_code: Option<i32>,
) -> Failure {
    let mut failure = unavailable();
    failure.details = Some(serde_json::json!({"stage":stage,"reason":reason,"osCode":os_code}));
    failure
}
pub(crate) fn io_failure(stage: &'static str, error: std::io::Error) -> Failure {
    let reason = match error.kind() {
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::AlreadyExists => "already_exists",
        _ => "io_error",
    };
    diagnostic(stage, reason, error.raw_os_error())
}
pub(crate) fn context(mut failure: Failure, area: &'static str) -> Failure {
    let details = failure.details.get_or_insert_with(
        || serde_json::json!({"stage":"validation","reason":"unavailable","osCode":null}),
    );
    details["area"] = serde_json::json!(area);
    failure
}

pub(crate) fn validate(path: &Path, directory: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink()
                || (directory && !meta.is_dir())
                || (!directory && !meta.is_file())
            {
                return Err(diagnostic("validation", "unsafe_file_type", None));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if meta.permissions().mode() & 0o077 != 0
                    || meta.uid() != unsafe { libc::geteuid() }
                {
                    return Err(diagnostic(
                        "validation",
                        "unsafe_permissions_or_owner",
                        None,
                    ));
                }
            }
            #[cfg(windows)]
            windows::validate(path)?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io_failure("metadata", e)),
    }
}

pub(crate) fn read_bytes(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let parent = path.parent().ok_or_else(unavailable)?;
    if !validate(parent, true)? || !validate(path, false)? {
        return Ok(None);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|e| io_failure("open_read", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = file.metadata().map_err(|e| io_failure("metadata", e))?;
        if !meta.is_file()
            || meta.permissions().mode() & 0o077 != 0
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return Err(diagnostic(
                "validation",
                "unsafe_permissions_or_owner",
                None,
            ));
        }
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_failure("read", e))?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(diagnostic("read", "size_limit", None));
    }
    Ok(Some(bytes))
}
pub(crate) fn read<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(None);
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| unavailable())?;
    let value = crate::input::strict_json(text).map_err(|_| unavailable())?;
    serde_json::from_value(value)
        .map(Some)
        .map_err(|_| unavailable())
}

pub(crate) fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_policy(path, value, true, MAX_METADATA)
}
/// Profile and catalog directories retain their existing policy and catalog size budget.
pub(crate) fn write_metadata<T: Serialize>(path: &Path, value: &T, limit: u64) -> Result<()> {
    write_policy(path, value, false, limit)
}
fn write_policy<T: Serialize>(path: &Path, value: &T, private: bool, limit: u64) -> Result<()> {
    let parent = path.parent().ok_or_else(unavailable)?;
    if private {
        directory(parent)?;
        validate(path, false)?;
    } else {
        metadata_directory(parent)?;
    }
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| unavailable())?);
    if bytes.len() as u64 > limit {
        return Err(diagnostic("serialize", "size_limit", None));
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| io_failure("create_temporary", e))?;
    let prepared = (|| {
        protect_new(temporary.path())?;
        temporary
            .write_all(&bytes)
            .map_err(|e| io_failure("write", e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| io_failure("sync_file", e))
    })();
    if let Err(failure) = prepared {
        return Err(cleanup_temporary(temporary, failure));
    }
    if let Err(error) = temporary.persist(path) {
        return Err(cleanup_temporary(
            error.file,
            io_failure("atomic_replace", error.error),
        ));
    }
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| io_failure("sync_directory", e))?;
    Ok(())
}

fn cleanup_temporary(temporary: tempfile::NamedTempFile, mut failure: Failure) -> Failure {
    let path = temporary.path().to_owned();
    if let Err(error) = temporary.close() {
        let details = failure.details.get_or_insert_with(|| serde_json::json!({}));
        details["cleanup"] =
            serde_json::json!({"status":"failed","path":path,"osCode":error.raw_os_error()});
    }
    failure
}

pub(crate) fn directory(parent: &Path) -> Result<()> {
    if !validate(parent, true)? {
        #[cfg(windows)]
        windows::create_directory(parent)?;
        #[cfg(not(windows))]
        {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(parent)
                .map_err(|e| io_failure("create_directory", e))?;
        }
    }
    validate(parent, true)?;
    Ok(())
}

/// Restrict only an object this caller just created; existing broad permissions are rejected.
pub(crate) fn protect_new(path: &Path) -> Result<()> {
    #[cfg(windows)]
    windows::protect_new(path)?;
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

fn metadata_directory(parent: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .map_err(|e| io_failure("create_directory", e))
}

/// Exercise this process's file policy with no credentials and no scan of existing entries.
pub(crate) fn probe(parent: &Path) -> Result<serde_json::Value> {
    probe_policy(parent, true)
}
pub(crate) fn probe_metadata(parent: &Path) -> Result<serde_json::Value> {
    probe_policy(parent, false)
}
fn probe_policy(parent: &Path, private: bool) -> Result<serde_json::Value> {
    if private {
        directory(parent)?;
    } else {
        metadata_directory(parent)?;
    }
    let destination = tempfile::Builder::new()
        .prefix(".dt-cli-probe-")
        .tempfile_in(parent)
        .map_err(|e| io_failure("create_temporary", e))?;
    if let Err(failure) = protect_new(destination.path()) {
        return Err(cleanup_temporary(destination, failure));
    }
    // Close the initial Windows handle before replacing the probe target.
    let destination = destination.into_temp_path();
    let path = destination.to_path_buf();
    let outcome = (|| {
        write_policy(
            &path,
            &serde_json::json!({"probe":true}),
            private,
            MAX_METADATA,
        )?;
        let bytes = fs::read(&path).map_err(|e| io_failure("read_back", e))?;
        let value: Option<serde_json::Value> = Some(
            serde_json::from_slice(&bytes)
                .map_err(|_| diagnostic("read_back", "decode_failed", None))?,
        );
        if value != Some(serde_json::json!({"probe":true})) {
            return Err(diagnostic("read_back", "mismatch", None));
        }
        Ok(())
    })();
    let cleanup = destination.close();
    if let Err(mut failure) = outcome {
        if let Err(error) = cleanup {
            let details = failure.details.get_or_insert_with(|| serde_json::json!({}));
            details["cleanup"] =
                serde_json::json!({"status":"failed","path":path,"osCode":error.raw_os_error()});
        }
        return Err(failure);
    }
    cleanup.map_err(|e| {
        let mut failure = io_failure("delete", e);
        failure.details.as_mut().unwrap()["cleanupPath"] = serde_json::json!(path);
        failure
    })?;
    Ok(
        serde_json::json!({"status":"verified","create":true,"write":true,"sync":true,"atomicReplace":true,"readBack":true,"delete":true}),
    )
}
