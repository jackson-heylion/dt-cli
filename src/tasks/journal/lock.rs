use crate::{Runtime, output::Result, private_store};
use fs2::FileExt;
use rand::RngCore;
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;

pub(crate) async fn acquire(path: &Path) -> Result<File> {
    let file = open(path)?;
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(crate::constants::storage::JOURNAL_LOCK_SECONDS);
    loop {
        if try_lock(&file)? {
            return Ok(file);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(private_store::unavailable());
        }
        tokio::time::sleep(Duration::from_millis(
            crate::constants::storage::LOCK_POLL_MILLIS,
        ))
        .await;
    }
}

/// Probe the same operating-system lock without waiting for another runner.
pub(crate) fn try_acquire(path: &Path) -> Result<Option<File>> {
    let file = open(path)?;
    Ok(if try_lock(&file)? { Some(file) } else { None })
}

fn try_lock(file: &File) -> Result<bool> {
    match file.try_lock_exclusive() {
        Ok(()) => Ok(true),
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(false)
        }
        Err(_) => Err(private_store::unavailable()),
    }
}

fn open(path: &Path) -> Result<File> {
    private_store::directory(path.parent().ok_or_else(private_store::unavailable)?)?;
    let existed = private_store::validate(path, false)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|_| private_store::unavailable())?;
    if !existed {
        private_store::protect_new(path)?;
    }
    private_store::validate(path, false)?;
    Ok(file)
}

pub(crate) async fn key(rt: &Runtime, create: bool) -> Result<Zeroizing<Vec<u8>>> {
    let _lock = acquire(&rt.root.join("workpacks/.key.lock")).await?;
    const NAME: &str = crate::constants::storage::JOURNAL_SIGNING_KEY;
    let key = match rt.store.read_integrity_key(NAME)? {
        Some(key) => Zeroizing::new(key),
        None if create => {
            let mut key = Zeroizing::new(vec![0u8; crate::constants::identity::RANDOM_BYTES]);
            rand::rngs::OsRng.fill_bytes(&mut key);
            rt.store.write_integrity_key(NAME, &key)?;
            key
        }
        None => return Err(private_store::unavailable()),
    };
    if key.len() != crate::constants::identity::RANDOM_BYTES {
        return Err(private_store::unavailable());
    }
    Ok(key)
}
