//! 浏览器启动与登录进度；启动调用成功不等于页面已显示或授权已完成。
use crate::output::{Failure, Result};
use serde::Serialize;
use std::{
    io::Write,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::net::TcpListener;

pub trait Browser: Send + Sync {
    fn open(&self, url: &str) -> Result<()>;
}
pub struct SystemBrowser;
impl Browser for SystemBrowser {
    fn open(&self, url: &str) -> Result<()> {
        webbrowser::open(url)
            .map_err(|_| Failure::new("BROWSER_OPEN_FAILED", 6, "浏览器未能启动。"))
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum BrowserLaunch {
    /// 尚未调用系统浏览器。
    NotAttempted,
    /// 系统打开调用返回成功，窗口与页面仍未确认。
    Requested,
    /// 本次命令显式禁止自动打开浏览器。
    Disabled,
    /// 系统打开调用返回失败。
    Failed,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CallbackStatus {
    /// 尚未收到本机端口连接。
    Waiting,
    /// 已收到连接，尚未取得有效授权回调。
    Connected,
    /// 回调状态、来源及授权码已通过校验。
    Validated,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginEvidence {
    /// 当前登录阶段，不代表页面已显示。
    stage: &'static str,
    /// 本次系统浏览器启动调用的实际结果。
    browser_launch: BrowserLaunch,
    /// 是否核实浏览器窗口；CLI 不观察桌面，始终为 false。
    browser_window_verified: bool,
    /// 本次监听的本机地址，不含授权参数。
    callback_address: String,
    /// 已观察到的连接或回调校验状态。
    callback_status: CallbackStatus,
    /// 本次取得有效授权回调的最长等待时间，单位：秒。
    wait_seconds: u64,
}

#[derive(Serialize)]
struct Progress<'a> {
    /// 登录进度事件名称；标准输出仍只包含最终信封。
    event: &'static str,
    /// 已发生的执行阶段。
    stage: &'a str,
}

#[derive(Serialize)]
struct BrowserProgress<'a> {
    /// 登录进度事件名称。
    event: &'static str,
    /// 本次登录的可核对事实。
    #[serde(flatten)]
    evidence: LoginEvidence,
    /// 当前需要本人完成的操作。
    message: &'static str,
    /// 本次 HTTPS 授权入口；仅等待期间提供，不写入失败结果。
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
}

fn emit(value: &impl Serialize) {
    let mut stderr = std::io::stderr().lock();
    if serde_json::to_writer(&mut stderr, value).is_ok() {
        let _ = writeln!(stderr);
    }
}

pub(crate) fn progress(stage: &'static str) {
    emit(&Progress {
        event: "auth.login.progress",
        stage,
    });
}

pub(crate) struct BrowserLogin {
    /// 本次系统浏览器启动结果。
    launch: BrowserLaunch,
    /// 本次已成功绑定的本机监听地址。
    address: SocketAddr,
    /// 是否已有连接到达；连接本身不证明授权通过。
    connected: AtomicBool,
    /// 有效授权回调等待期限，单位：秒。
    wait_seconds: u64,
}

impl BrowserLogin {
    pub(crate) fn begin(
        browser: &dyn Browser,
        url: &str,
        address: SocketAddr,
        disabled: bool,
    ) -> Self {
        let mut login = Self {
            launch: BrowserLaunch::NotAttempted,
            address,
            connected: AtomicBool::new(false),
            wait_seconds: 300,
        };
        login.launch = if disabled {
            BrowserLaunch::Disabled
        } else {
            login.emit("opening_browser", CallbackStatus::Waiting, None);
            match browser.open(url) {
                Ok(()) => BrowserLaunch::Requested,
                Err(_) => BrowserLaunch::Failed,
            }
        };
        let stage = if matches!(login.launch, BrowserLaunch::Failed) {
            "open_browser_failed"
        } else {
            "waiting_for_browser"
        };
        login.emit(stage, CallbackStatus::Waiting, Some(url));
        login
    }

    fn evidence(&self, stage: &'static str, callback_status: CallbackStatus) -> LoginEvidence {
        LoginEvidence {
            stage,
            browser_launch: self.launch,
            browser_window_verified: false,
            callback_address: self.address.to_string(),
            callback_status,
            wait_seconds: self.wait_seconds,
        }
    }

    fn emit(&self, stage: &'static str, status: CallbackStatus, url: Option<&str>) {
        let message = match stage {
            "opening_browser" => "正在请求打开系统浏览器。",
            "waiting_for_browser" if matches!(self.launch, BrowserLaunch::Disabled) => {
                "自动打开已禁用，请打开本次登录链接并完成验证。"
            }
            "waiting_for_browser" => {
                "请在浏览器完成登录验证；未看到窗口时打开本次登录链接，登录成功后自动继续。"
            }
            "open_browser_failed" => "未能自动打开浏览器，请打开本次登录链接并完成验证。",
            "callback_connected" => "本机端口已收到连接，正在等待完整授权回调。",
            "callback_received" => "授权回调已通过校验，正在继续登录。",
            _ => "本次等待已结束，登录未完成。",
        };
        emit(&BrowserProgress {
            event: "auth.login.progress",
            evidence: self.evidence(stage, status),
            message,
            url,
        });
    }

    pub(super) fn callback_connected(&self) {
        self.connected.store(true, Ordering::Relaxed);
        self.emit("callback_connected", CallbackStatus::Connected, None);
    }

    pub(crate) async fn wait(
        &self,
        listener: &TcpListener,
        state: &str,
        origin: &str,
    ) -> Result<String> {
        let callback = super::callback_observed(listener, state, origin, Some(self));
        match tokio::time::timeout(Duration::from_secs(self.wait_seconds), callback).await {
            Ok(result) => {
                if result.is_ok() {
                    self.emit("callback_received", CallbackStatus::Validated, None);
                }
                result
            }
            Err(_) => {
                let status = if self.connected.load(Ordering::Relaxed) {
                    CallbackStatus::Connected
                } else {
                    CallbackStatus::Waiting
                };
                self.emit("timed_out", status, None);
                let mut failure = Failure::new(
                    "TIMEOUT",
                    5,
                    "浏览器登录未完成，未在等待期限内取得有效授权回调。",
                );
                failure.retryable = false;
                failure.hint = if self.connected.load(Ordering::Relaxed) {
                    "已收到本机连接，但未取得完整有效的授权回调；请核对浏览器页面和回调过程。当前登录已结束，原链接不再可用。"
                } else {
                    "未收到本机回调；请核对浏览器是否显示登录页、页面加载与本机回调可达性。当前登录已结束，原链接不再可用。"
                };
                let evidence = self.evidence("timed_out", status);
                failure.details = Some(crate::output::Details {
                    stage: Some(evidence.stage.into()),
                    browser_launch: Some(
                        match evidence.browser_launch {
                            BrowserLaunch::NotAttempted => "not_attempted",
                            BrowserLaunch::Requested => "requested",
                            BrowserLaunch::Disabled => "disabled",
                            BrowserLaunch::Failed => "failed",
                        }
                        .into(),
                    ),
                    browser_window_verified: Some(false),
                    callback_address: Some(evidence.callback_address),
                    callback_status: Some(
                        match status {
                            CallbackStatus::Waiting => "waiting",
                            CallbackStatus::Connected => "connected",
                            CallbackStatus::Validated => "validated",
                        }
                        .into(),
                    ),
                    wait_seconds: Some(evidence.wait_seconds),
                    ..Default::default()
                });
                Err(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests;
