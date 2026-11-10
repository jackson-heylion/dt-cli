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
        // 使用与原生存储相同的当前 SID、完整 DACL 替换与即时校验，不保留复制来源的 ACE。
        dt_cli::protect_created_fixture(path).unwrap_or_else(|failure| {
            panic!("夹具私有策略设置失败 {}: {failure:?}", path.display())
        });
    }
}
