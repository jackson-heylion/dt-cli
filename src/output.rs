mod details;
mod partial;
mod recovery;
pub use details::{Cleanup, Details, RenameComparison};
pub(crate) use partial::{Data as PartialData, Meta as PartialMeta};
pub use recovery::{Action as RecoveryAction, Recovery};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
pub struct Failure {
    /// 稳定客户端错误代码。
    pub code: &'static str,
    /// 稳定故障类别，与请求重发许可独立。
    pub category: &'static str,
    /// 客户端固定安全说明，不透传服务器原文。
    pub message: &'static str,
    /// 仅表示原请求可安全重发；未知写入为 false。
    pub retryable: bool,
    /// 固定安全处理提示。
    pub hint: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// 经过协议白名单验证的具名诊断字段，可空。
    pub details: Option<Details>,
    #[serde(skip)]
    /// 原生命令退出码。
    pub exit: u8,
    #[serde(skip)]
    /// 绑定原本人、环境和记录的具名恢复动作，可空。
    pub recovery: Option<Recovery>,
    #[serde(skip)]
    /// 经字符与长度核验的关联号，可空。
    pub trace_id: Option<String>,
    /// Records already retrieved when a bounded aggregation stops early.
    #[serde(skip)]
    pub(crate) partial_data: Option<Box<PartialData>>,
    /// Completeness metadata that must travel with `partial_data` in the same envelope.
    #[serde(skip)]
    pub(crate) partial_meta: Option<Box<PartialMeta>>,
}
impl Failure {
    pub fn new(code: &'static str, exit: u8, message: &'static str) -> Self {
        Self {
            code,
            details: None,
            exit,
            message,
            category: match exit {
                2 => "argument",
                3 | 4 | 8 => "auth",
                5 => "dependency",
                _ => "runtime",
            },
            retryable: false,
            hint: match exit {
                3 => "请执行 dt-cli login --profile <名称>，沿用原任务环境。",
                2 => "使用 dt-cli help 查看命令，describe 查看固定契约。",
                _ => "检查当前环境和平台会话；不要导入门户 token。",
            },
            recovery: None,
            trace_id: None,
            partial_data: None,
            partial_meta: None,
        }
    }
    /// A failed envelope may still carry a partial result; data and meta stay together.
    pub(crate) fn with_partial(mut self, data: PartialData, meta: PartialMeta) -> Self {
        self.partial_data = Some(Box::new(data));
        self.partial_meta = Some(Box::new(meta));
        self
    }
}

pub type Result<T> = std::result::Result<T, Failure>;
pub fn invalid() -> Failure {
    Failure::new("INVALID_ARGUMENT", 2, "参数不符合命令合同。")
}
pub fn storage() -> Failure {
    Failure::new(
        "CREDENTIAL_STORE_UNAVAILABLE",
        1,
        "本地凭证目录不可用或权限不安全；未继续执行。",
    )
}
/// 最终输出边界；动态 JSON 仅用于信封序列化，不作为业务模型。
pub fn envelope_result(op: &str, profile: Option<&str>, result: Result<Value>) -> (Value, u8) {
    match result {
        Ok(data) => (
            json!({"schemaVersion":crate::constants::protocol::ENVELOPE_SCHEMA,"operationId":op,"ok":true,"profile":profile,"data":data,"error":null,"meta":{}}),
            0,
        ),
        Err(error) => {
            let mut meta = error
                .partial_meta
                .as_deref()
                .map(|v| serde_json::to_value(v).expect("具名元数据序列化"))
                .unwrap_or_else(|| json!({}));
            meta["traceId"] = json!(error.trace_id);
            meta["recovery"] = json!(error.recovery);
            let data = error
                .partial_data
                .as_deref()
                .map(|v| serde_json::to_value(v).expect("具名结果序列化"))
                .unwrap_or(Value::Null);
            let code = error.exit;
            (
                json!({"schemaVersion":crate::constants::protocol::ENVELOPE_SCHEMA,"operationId":op,"ok":false,"profile":profile,"data":data,"error":error,"meta":meta}),
                code,
            )
        }
    }
}
pub fn render(v: &Value, table: bool) -> String {
    if !table {
        return serde_json::to_string(v).unwrap();
    }
    // Preserve the full result, including errors and diagnostic scope, in human-readable rows.
    fn rows(v: &Value, prefix: &str, out: &mut String) {
        if let Some(obj) = v.as_object() {
            for (k, v) in obj {
                rows(
                    v,
                    &if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    },
                    out,
                );
            }
        } else {
            out.push_str(&format!("{prefix}\t{v}\n"));
        }
    }
    let mut out = String::new();
    rows(v, "", &mut out);
    out
}
