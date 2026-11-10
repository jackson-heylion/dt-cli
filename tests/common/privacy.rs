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
        // 提权运行器新建对象可能归 Administrators 所有；夹具须与当前用户私有策略一致。
        for arguments in [
            vec!["/setowner", user.as_str(), "/q"],
            vec!["/inheritance:r", "/grant:r", permission.as_str(), "/q"],
        ] {
            let status = std::process::Command::new("icacls.exe")
                .arg(path)
                .args(arguments)
                .stdout(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "夹具所有权或权限设置失败");
        }
    }
}
