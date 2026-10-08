use super::*;

pub(super) fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if fs::symlink_metadata(path)
        .map_err(|_| io_error())?
        .file_type()
        .is_symlink()
    {
        return Err(io_error());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| io_error())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| io_error())?;
    if bytes.len() as u64 > limit {
        return Err(bad_package());
    }
    Ok(bytes)
}
pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(io_error)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|_| io_error())?;
    file.write_all(bytes).map_err(|_| io_error())?;
    file.as_file().sync_all().map_err(|_| io_error())?;
    let temp = file.into_temp_path();
    atomic_replace(&temp, path).map_err(|_| io_error())?;
    #[cfg(unix)]
    {
        let _ = File::open(parent).and_then(|f| f.sync_all());
    }
    Ok(())
}
#[cfg(not(target_os = "windows"))]
pub(super) fn atomic_replace(source: &Path, dest: &Path) -> std::io::Result<()> {
    fs::rename(source, dest)
}
#[cfg(target_os = "windows")]
pub(super) fn atomic_replace(source: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dest: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
    // Both paths are owned, NUL-terminated buffers alive throughout this call.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            dest.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub(super) fn plain_directory(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path)
        && (meta.file_type().is_symlink() || !meta.is_dir())
    {
        return Err(io_error());
    }
    fs::create_dir_all(path).map_err(|_| io_error())
}
pub(super) struct UpdateLock(File);
impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub(super) fn lock(root: &Path) -> Result<UpdateLock> {
    let parent = root.parent().ok_or_else(io_error)?;
    plain_directory(parent)?;
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink() || !m.is_dir()) {
        return Err(io_error());
    }
    let name = root.file_name().ok_or_else(io_error)?.to_string_lossy();
    let path = parent.join(format!(".{name}.dt-cli-update.lock"));
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(io_error());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| io_error())?;
    file.try_lock_exclusive()
        .map_err(|_| failure("UPGRADE_BUSY", "另一进程正在安装或升级；请稍后重试。"))?;
    Ok(UpdateLock(file))
}

pub(super) trait Verifier {
    fn probe(&self, binary: &Path) -> Result<Value>;
}
