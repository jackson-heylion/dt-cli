//! 只保护测试调用自己创建的夹具文件，符合当前平台私有存储策略。
pub fn protect(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if path.is_dir() { 0o700 } else { 0o600 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap();
        let permission = if path.is_dir() {
            format!("{user}:(OI)(CI)F")
        } else {
            format!("{user}:F")
        };
        let status = std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/inheritance:r", "/grant:r", &permission, "/q"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "夹具权限设置失败");
    }
}
