use crate::{Runtime, output::Result, private_store};
use fs2::FileExt;
use rand::RngCore;
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;

pub(super) async fn acquire(path: &Path) -> Result<File> {
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(private_store::unavailable());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(_) => return Err(private_store::unavailable()),
        }
    }
}

pub(super) async fn key(rt: &Runtime, create: bool) -> Result<Zeroizing<Vec<u8>>> {
    let _lock = acquire(&rt.root.join("workpacks/.key.lock")).await?;
    const NAME: &str = "metadata/workpack-signing-v1";
    let key = match rt.store.read_integrity_key(NAME)? {
        Some(key) => Zeroizing::new(key),
        None if create => {
            let mut key = Zeroizing::new(vec![0u8; 32]);
            rand::rngs::OsRng.fill_bytes(&mut key);
            rt.store.write_integrity_key(NAME, &key)?;
            key
        }
        None => return Err(private_store::unavailable()),
    };
    if key.len() != 32 {
        return Err(private_store::unavailable());
    }
    Ok(key)
}
