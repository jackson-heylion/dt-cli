//! 本机目录、名称与会话协调；不读取旧 profile 或环境目录。
use crate::output::{Failure, Result, invalid, storage};
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub fn config_dir() -> Result<PathBuf> {
    directories::ProjectDirs::from(
        crate::constants::identity::QUALIFIER,
        crate::constants::identity::ORGANIZATION,
        crate::constants::identity::CLIENT_ID,
    )
    .map(|p| p.config_dir().to_owned())
    .ok_or_else(storage)
}
pub fn validate_name(name: &str) -> Result<()> {
    if name.starts_with("dtp_")
        || name.starts_with("dtcli_")
        || name.is_empty()
        || name.len() > crate::constants::identity::MAX_ID_LENGTH
        || name.chars().any(|c| c.is_control() || "/\\:".contains(c))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
pub struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub async fn lock(root: &Path, name: &str) -> Result<Lock> {
    validate_name(name)?;
    fs::create_dir_all(root).map_err(|e| {
        crate::private_store::context(
            crate::private_store::io_failure("create_directory", e),
            "profile_lock",
        )
    })?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(format!("{name}.lock")))
        .map_err(|e| {
            crate::private_store::context(
                crate::private_store::io_failure("open_lock", e),
                "profile_lock",
            )
        })?;
    let start = Instant::now();
    loop {
        match f.try_lock_exclusive() {
            Ok(()) => return Ok(Lock(f)),
            Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(e) => {
                return Err(crate::private_store::context(
                    crate::private_store::io_failure("lock", e),
                    "profile_lock",
                ));
            }
        }
        if start.elapsed() >= Duration::from_secs(crate::constants::storage::LOCK_SECONDS) {
            return Err(Failure::new("TIMEOUT", 5, "等待 profile 锁超时。"));
        }
        tokio::time::sleep(Duration::from_millis(
            crate::constants::storage::LOCK_POLL_MILLIS,
        ))
        .await;
    }
}
pub async fn authorization_lock(root: &Path, authorization_id: &str) -> Result<Lock> {
    use sha2::{Digest, Sha256};
    let key = format!("auth-{:x}", Sha256::digest(authorization_id.as_bytes()));
    lock(root, &key).await
}
