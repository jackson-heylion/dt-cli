use crate::{credentials, login};
use std::{path::PathBuf, time::Duration};

pub struct Runtime {
    /// 本机配置与平台记录根目录。
    pub root: PathBuf,
    /// 安全凭证与完整性密钥存储。
    pub store: Box<dyn credentials::CredentialStore>,
    /// 本机浏览器启动能力。
    pub browser: Box<dyn login::Browser>,
    /// 当前终端是否可交互。
    pub interactive: bool,
    /// 单次有界任务的绝对预算，单位：秒；含锁等待及续期。
    pub aggregate_budget: Duration,
}
