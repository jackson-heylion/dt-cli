mod tasks;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: &'static str,
    pub category: &'static str,
    pub message: &'static str,
    pub retryable: bool,
    pub hint: &'static str,
    #[serde(skip)]
    pub exit: u8,
    #[serde(skip)]
    pub recovery: Option<Box<Value>>,
    #[serde(skip)]
    pub trace_id: Option<String>,
    /// Records already retrieved when a bounded aggregation stops early.
    #[serde(skip)]
    pub partial_data: Option<Box<Value>>,
    /// Completeness metadata that must travel with `partial_data` in the same envelope.
    #[serde(skip)]
    pub partial_meta: Option<Box<Value>>,
}
impl Failure {
    pub fn new(code: &'static str, exit: u8, message: &'static str) -> Self {
        Self {
            code,
            exit,
            message,
            category: match exit {
                2 => "argument",
                3 | 4 | 8 => "auth",
                5 => "dependency",
                _ => "runtime",
            },
            retryable: exit == 5,
            hint: match exit {
                3 => {
                    "请在交互终端显式执行 dt-cli auth login --profile <名称>；新 profile 还需 --environment。"
                }
                2 => "使用 dt-cli help 或 schema 查看合同。",
                _ => "检查环境或通过 IAM 门户处理独立 CLI 授权；不要导入门户 token。",
            },
            recovery: None,
            trace_id: None,
            partial_data: None,
            partial_meta: None,
        }
    }
    /// A failed envelope may still carry a partial result; data and meta stay together.
    pub fn with_partial(mut self, data: Value, meta: Value) -> Self {
        self.partial_data = Some(Box::new(data));
        self.partial_meta = Some(Box::new(meta));
        self
    }
}

/// Internal execution result. Unlike the public envelope it can hold a failure *and* the
/// records already retrieved, so partial aggregation is reported without recomputing anything.
pub struct Executed {
    pub data: Value,
    pub meta: Value,
    pub error: Option<Failure>,
}
impl Executed {
    pub fn ok(data: Value) -> Self {
        Self {
            data,
            meta: json!({}),
            error: None,
        }
    }
    pub fn ok_with_meta(data: Value, meta: Value) -> Self {
        Self {
            data,
            meta,
            error: None,
        }
    }
    pub fn failed(error: Failure) -> Self {
        Self {
            data: Value::Null,
            meta: json!({}),
            error: Some(error),
        }
    }
    /// Shared executor values still mark their meta with `_meta`/`_traceId`.
    pub fn value(value: Value) -> Self {
        split(value)
    }
}
fn split(mut value: Value) -> Executed {
    let internal_meta = value.as_object_mut().and_then(|o| o.remove("_meta"));
    let trace = value
        .as_object_mut()
        .and_then(|o| o.remove("_traceId"))
        .unwrap_or(Value::Null);
    let mut meta = internal_meta.unwrap_or_else(|| json!({}));
    if let Some(object) = meta.as_object_mut()
        && !trace.is_null()
    {
        object.insert("traceId".into(), trace);
    }
    Executed {
        data: value,
        meta,
        error: None,
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
pub fn envelope(op: &str, profile: Option<&str>, executed: Executed) -> (Value, u8) {
    match executed.error {
        None => (
            json!({"schemaVersion":"1.0","operationId":op,"ok":true,"profile":profile,
                "data":executed.data,"error":Value::Null,"meta":executed.meta}),
            0,
        ),
        Some(e) => {
            let mut meta = e
                .partial_meta
                .as_deref()
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Some(object) = meta.as_object_mut() {
                if !object.contains_key("traceId") {
                    object.insert("traceId".into(), json!(e.trace_id));
                }
                object.insert("recovery".into(), json!(e.recovery));
            }
            let data = e.partial_data.as_deref().cloned().unwrap_or(Value::Null);
            let exit = e.exit;
            (
                json!({"schemaVersion":"1.0","operationId":op,"ok":false,"profile":profile,
                    "data":data,"error":serde_json::to_value(&e).unwrap(),"meta":meta}),
                exit,
            )
        }
    }
}
pub fn envelope_result(op: &str, profile: Option<&str>, result: Result<Value>) -> (Value, u8) {
    envelope(
        op,
        profile,
        match result {
            Ok(value) => Executed::value(value),
            Err(e) => Executed::failed(e),
        },
    )
}
pub fn render(v: &Value, table: bool) -> String {
    if !table {
        return serde_json::to_string(v).unwrap();
    }
    if let Some(rendered) = tasks::render(v) {
        return rendered;
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
