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
/// 唯一命令入口；业务只通过当前平台与 Gateway 执行。
pub async fn execute(rt: &Runtime, args: Vec<String>) -> (serde_json::Value, u8, bool) {
    interface::execute(rt, args).await
}
