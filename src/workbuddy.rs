//! Explicit, directory-scoped WorkBuddy configuration; never part of a business read.
use crate::output::{Failure, Result};
use fs2::FileExt;
use serde_json::json;
use std::{
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};

#[cfg(windows)]
mod windows;

pub(crate) fn doctor(root: &Path, fix: bool) -> Result<serde_json::Value> {
    #[cfg(any(target_os = "macos", windows))]
    {
        let dirs = directories::BaseDirs::new().ok_or_else(|| failure("home"))?;
        let project = directories::ProjectDirs::from("com", "datousoft", "dt-cli")
            .ok_or_else(|| failure("directory"))?;
        let mut roots = vec![root.to_owned()];
        let data = project.data_local_dir();
        if data != root {
            roots.push(data.to_owned());
        }
        configure(dirs.home_dir(), &roots, fix)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (root, fix);
        Err(Failure::new(
            "WORKBUDDY_PLATFORM_UNSUPPORTED",
            2,
            "此目录配置入口用于 macOS 和 Windows WorkBuddy。",
        ))
    }
}

fn failure(stage: &str) -> Failure {
    let mut error = Failure::new(
        "WORKBUDDY_CONFIG_UNAVAILABLE",
        1,
        "无法安全更新 WorkBuddy 配置；原设置未被替换。",
    );
    error.hint = "在 WorkBuddy 文件权限设置中允许 dt-cli 数据目录读、写和删除。";
    error.details = Some(serde_json::json!({"stage":stage}));
    error
}

fn configure(home: &Path, roots: &[PathBuf], fix: bool) -> Result<serde_json::Value> {
    const LIMIT: u64 = 1024 * 1024;
    let parent = home.join(".workbuddy");
    let settings = parent.join("settings.json");
    if roots.is_empty() || roots.iter().any(|root| !root.is_absolute()) {
        return Err(failure("directory"));
    }
    validate_parent(&parent)?;
    let open = || open_settings(&settings);
    let read = |file: &fs::File| -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure("read"))?;
        if bytes.len() as u64 > LIMIT {
            return Err(failure("size_limit"));
        }
        Ok(bytes)
    };
    let (mut file, meta) = open()?;
    if fix {
        file.try_lock_exclusive().map_err(|_| failure("busy"))?;
    }
    let before = read(&file)?;
    let mut value = std::str::from_utf8(&before)
        .ok()
        .map(|text| text.strip_prefix('\u{feff}').unwrap_or(text))
        .and_then(|text| crate::input::strict_json(text).ok())
        .filter(|value| value.is_object())
        .ok_or_else(|| failure("decode"))?;
    let object = value.as_object_mut().unwrap();
    let sandbox = object.entry("sandbox").or_insert_with(|| json!({}));
    let sandbox = sandbox.as_object_mut().ok_or_else(|| failure("decode"))?;
    let paths = sandbox
        .entry("extraAllowWrite")
        .or_insert_with(|| json!([]));
    let paths = paths.as_array_mut().ok_or_else(|| failure("decode"))?;
    if paths.iter().any(|path| !path.is_string()) {
        return Err(failure("decode"));
    }
    let missing: Vec<_> = roots
        .iter()
        .filter(|root| {
            !paths
                .iter()
                .any(|path| matches_directory(home, path.as_str().unwrap(), root))
        })
        .collect();
    let configured = missing.is_empty();
    let changed = fix && !configured;
    if changed {
        for root in missing {
            paths.push(json!(root));
        }
        let mut bytes = serde_json::to_vec_pretty(&value).map_err(|_| failure("serialize"))?;
        bytes.push(b'\n');
        if bytes.len() as u64 > LIMIT {
            return Err(failure("size_limit"));
        }
        let mut temporary =
            tempfile::NamedTempFile::new_in(&parent).map_err(|_| failure("create_temporary"))?;
        #[cfg(unix)]
        temporary
            .as_file()
            .set_permissions(meta.permissions())
            .map_err(|_| failure("permissions"))?;
        temporary.write_all(&bytes).map_err(|_| failure("write"))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| failure("sync"))?;
        // WorkBuddy is another writer; retain a newer file rather than replacing it.
        let (_, current_meta) = open()?;
        // Read with the locked handle; Windows byte-range locks reject a second reader.
        file.rewind().map_err(|_| failure("read"))?;
        if current_meta != meta || read(&file)? != before {
            return Err(failure("settings_changed"));
        }
        replace_settings(temporary, &settings)?;
    }
    Ok(json!({
        "client":"WorkBuddy", "settingsFile":settings,
        "settingsKey":"sandbox.extraAllowWrite", "allowDirectory":roots[0], "allowDirectories":roots,
        "configured":configured || changed, "changed":changed,
        "scope":"WorkBuddy 中执行的程序可读、写、重命名和删除此 dt-cli 数据目录",
        "storageVerified":false,
        "nextAction":if configured || changed {"restart-workbuddy-then-verify"} else {"configure-directory"},
        "nextCommand":if configured || changed {
            json!(["dt-cli","doctor","--storage"])
        } else {
            json!(["dt-cli","doctor","--workbuddy","--fix"])
        },
        "hint":"configured 仅表示配置文件已保存，未验证当前进程权限。保存后完全退出并重新打开 WorkBuddy，再在新会话执行 doctor --storage。"
    }))
}

fn matches_directory(home: &Path, value: &str, root: &Path) -> bool {
    let expanded = if let Some(path) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        home.join(path)
    } else {
        PathBuf::from(value)
    };
    #[cfg(windows)]
    {
        expanded
            .to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .eq_ignore_ascii_case(
                root.to_string_lossy()
                    .replace('/', "\\")
                    .trim_end_matches('\\'),
            )
    }
    #[cfg(not(windows))]
    {
        expanded == root
    }
}

#[cfg(unix)]
fn validate_parent(parent: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(parent).map_err(|_| failure("settings"))?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o022 != 0
    {
        return Err(failure("settings"));
    }
    Ok(())
}
#[cfg(unix)]
#[derive(PartialEq)]
struct Snapshot {
    ino: u64,
    dev: u64,
    mode: u32,
    permissions: fs::Permissions,
}
#[cfg(unix)]
impl Snapshot {
    fn permissions(&self) -> fs::Permissions {
        self.permissions.clone()
    }
}
#[cfg(unix)]
fn open_settings(path: &Path) -> Result<(fs::File, Snapshot)> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| failure("settings"))?;
    let meta = file.metadata().map_err(|_| failure("settings"))?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
        return Err(failure("settings"));
    }
    Ok((
        file,
        Snapshot {
            ino: meta.ino(),
            dev: meta.dev(),
            mode: meta.mode(),
            permissions: meta.permissions(),
        },
    ))
}
#[cfg(unix)]
fn replace_settings(temporary: tempfile::NamedTempFile, path: &Path) -> Result<()> {
    temporary
        .persist(path)
        .map_err(|_| failure("atomic_replace"))?;
    Ok(())
}
#[cfg(windows)]
use windows::{open_settings, replace_settings, validate_parent};

#[cfg(test)]
mod tests;
