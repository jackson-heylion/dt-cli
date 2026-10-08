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

pub(crate) fn validate(path: &Path, directory: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink()
                || (directory && !meta.is_dir())
                || (!directory && !meta.is_file())
            {
                return Err(unavailable());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if meta.permissions().mode() & 0o077 != 0
                    || meta.uid() != unsafe { libc::geteuid() }
                {
                    return Err(unavailable());
                }
            }
            #[cfg(windows)]
            windows::validate(path)?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(unavailable()),
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
    let file = options.open(path).map_err(|_| unavailable())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let meta = file.metadata().map_err(|_| unavailable())?;
        if !meta.is_file()
            || meta.permissions().mode() & 0o077 != 0
            || meta.uid() != unsafe { libc::geteuid() }
        {
            return Err(unavailable());
        }
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(unavailable());
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
    let parent = path.parent().ok_or_else(unavailable)?;
    directory(parent)?;
    validate(path, false)?;
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| unavailable())?);
    if bytes.len() as u64 > MAX_METADATA {
        return Err(unavailable());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| unavailable())?;
    protect_new(temporary.path())?;
    temporary.write_all(&bytes).map_err(|_| unavailable())?;
    temporary.as_file().sync_all().map_err(|_| unavailable())?;
    temporary.persist(path).map_err(|_| unavailable())?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| unavailable())?;
    Ok(())
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
            builder.create(parent).map_err(|_| unavailable())?;
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
