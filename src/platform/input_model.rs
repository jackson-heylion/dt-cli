use serde::{Deserialize, Serialize};

/// 可配置接口的参数是显式传输标量，不作为无类型业务对象跨层传递。
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum Scalar {
    Text(String),
    Boolean(bool),
    Number(serde_json::Number),
    Null,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum ArgumentValue {
    Scalar(Scalar),
    Array(Vec<Scalar>),
    Object(ObjectArgument),
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObjectArgument {
    /// 以具名属性列表表达需要序列化的参数对象。
    pub properties: Vec<NamedScalar>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NamedScalar {
    /// 参数对象内的属性名。
    pub name: String,
    /// 属性的传输标量。
    pub value: Scalar,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Argument {
    /// 契约声明的参数名称。
    pub name: String,
    /// 明确的标量、数组或属性列表。
    pub value: ArgumentValue,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskView {
    /// AUTH 返回的业务视角标识。
    pub view_id: u32,
    /// 本任务已选择的业务对象标识。
    pub object_id: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InvocationInput {
    /// 路径参数；由固定契约模板构建真实路径。
    #[serde(default)]
    pub path: Vec<Argument>,
    /// 查询参数，按契约风格序列化。
    #[serde(default)]
    pub query: Vec<Argument>,
    /// 普通业务头，不接收平台/业务身份头。
    #[serde(default)]
    pub headers: Vec<Argument>,
    /// 普通表单参数。
    #[serde(default)]
    pub form: Vec<Argument>,
    /// 普通 JSON 请求体原文，可用 --body-file 加载。
    pub body_json: Option<String>,
    /// 本任务明确选择的视角与对象；后续不跟随 BOH 当前选择。
    pub view: Option<TaskView>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BusinessHeader {
    /// 已声明的普通业务头名。
    pub name: String,
    /// 实际发送的业务头值。
    pub value: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WireInvocation {
    /// 与 Gateway 核验完全一致的实际路径。
    pub path: String,
    /// 实际编码后的原查询串。
    pub query: String,
    /// 明确声明的普通业务头。
    pub headers: Vec<BusinessHeader>,
    /// 请求体 UTF-8 原文，不包含任何受服务端管理的业务凭证。
    pub body: String,
    /// 原任务固定业务视角，可空。
    pub view: Option<TaskView>,
}
