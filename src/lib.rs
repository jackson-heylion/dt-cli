mod constants;
pub mod credentials;
pub mod http;
mod interface;
mod json_decode;
pub mod login;
pub mod output;
mod platform;
mod private_store;
pub mod profile;
mod record_lock;
pub mod release;
mod runtime;
mod workbuddy;
pub use runtime::Runtime;

/// 仅开发构建用于初始化测试自己创建的夹具；正式程序不提供此入口。
#[cfg(all(windows, feature = "local-dev"))]
#[doc(hidden)]
pub fn protect_created_fixture(path: &std::path::Path) -> output::Result<()> {
    private_store::protect_new(path)
}

/// 唯一命令入口；业务只通过当前平台与 Gateway 执行。
pub async fn execute(rt: &Runtime, args: Vec<String>) -> (serde_json::Value, u8, bool) {
    let (value, exit, table) = interface::execute(rt, args, None).await;
    (
        value.expect("non-streaming execution always returns a record"),
        exit,
        table,
    )
}

/// 原生命令入口，逐条写出流式记录，普通调用返回现有单条结果。
pub async fn execute_with_stream(
    rt: &Runtime,
    args: Vec<String>,
    stdout: &mut (dyn tokio::io::AsyncWrite + Unpin),
) -> (Option<serde_json::Value>, u8, bool) {
    interface::execute(rt, args, Some(stdout)).await
}
