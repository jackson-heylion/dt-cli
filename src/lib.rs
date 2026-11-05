pub mod catalog;
mod constants;
pub mod credentials;
pub mod governed;
pub mod http;
mod input;
pub mod login;
pub mod open;
pub mod output;
pub mod pagination;
mod platform;
pub mod profile;
pub mod release;

mod commands;
mod parameters;
mod private_store;
mod profiles;
mod recovery;
mod runtime;
mod tasks;
mod workbuddy;
/// 按独立平台命名空间分派，保留既有个人与受控命令行为。
pub async fn execute(rt: &Runtime, args: Vec<String>) -> (serde_json::Value, u8, bool) {
    if platform::selected(&args) {
        platform::execute(rt, args).await
    } else {
        commands::execute(rt, args).await
    }
}
pub use parameters::WorkflowPlan;
pub use runtime::Runtime;
