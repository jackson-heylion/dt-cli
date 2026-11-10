use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionContract {
    /// 执行契约格式版本。
    pub contract_schema_version: u32,
    /// 稳定接口标识。
    pub operation_id: String,
    /// 固定环境内执行版本。
    pub operation_version: u64,
    /// 最低客户端具体版本。
    pub minimum_cli_version: String,
    /// 固定业务系统。
    pub system_id: String,
    /// 固定运行环境。
    pub environment: String,
    /// 接口名称。
    pub title: String,
    /// 实际 HTTP 方法。
    pub method: Method,
    /// 已发布路径模板。
    pub path: String,
    /// 明确业务读写性质。
    pub effect: Effect,
    /// 普通请求体格式。
    pub media: Media,
    /// 已确认的参数清单。
    pub parameters: Vec<Parameter>,
    /// 请求 Schema 原文，可空。
    pub request_schema: Option<String>,
    /// 业务响应 Schema 原文，可空。
    pub response_schema: Option<String>,
    /// 业务成功条件。
    pub success: Success,
    /// 已确认分页规则。
    pub pagination: Pagination,
    /// 允许的业务视角。
    pub view_types: Vec<ViewType>,
    /// 授权数据范围。
    pub data_scope: String,
    /// 已确认的敏感字段。
    pub sensitive_fields: Vec<String>,
    /// 服务端路由绑定摘要。
    pub route_binding_digest: String,
    /// 工作包硬限额。
    pub limits: Limits,
    /// 请求体是否必须提供。
    pub request_body_required: bool,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}
impl Method {
    pub fn text(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        }
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Effect {
    Read,
    Write,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Media {
    None,
    Json,
    Form,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum Location {
    Path,
    Query,
    Header,
    Form,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum ViewType {
    Group,
    Brand,
    Delivery,
    Shop,
    Supplier,
    Manufacture,
    Franchise,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Parameter {
    /// 参数名称。
    pub name: String,
    /// 实际传输位置。
    pub location: Location,
    /// 是否必填。
    pub required: bool,
    /// 参数 Schema 原文。
    pub schema: Option<String>,
    /// OpenAPI 参数序列化风格。
    pub style: Option<String>,
    /// 是否展开数组对象。
    pub explode: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Success {
    /// 可接受的 HTTP 状态码。
    pub status_codes: Vec<u16>,
    /// 业务成功字段 JSON Pointer，可空。
    pub body_pointer: Option<String>,
    /// 业务成功期望值 JSON 原文，可空。
    pub expected_json: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pagination {
    /// NONE、PAGE 或 CURSOR。
    pub kind: PaginationKind,
    /// 页号或游标参数名，可空。
    pub page_parameter: Option<String>,
    /// 每页条数参数名，可空。
    pub size_parameter: Option<String>,
    /// 业务列表的 JSON Pointer。
    pub items_pointer: Option<String>,
    /// 下一页字段的 JSON Pointer，可空。
    pub next_pointer: Option<String>,
    /// 总条数字段的 JSON Pointer，可空。
    pub total_pointer: Option<String>,
    /// 第一页页号。
    pub first_page: u64,
    /// 默认每页条数。
    pub page_size: u32,
    /// 分页参数位于查询、表单或 JSON 请求体。
    pub placement: PaginationPlacement,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum PaginationPlacement {
    Query,
    Form,
    JsonBody,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub(crate) enum PaginationKind {
    None,
    Page,
    Cursor,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Limits {
    /// 最大页数。
    pub max_pages: u32,
    /// 最大记录数。
    pub max_records: u64,
    /// 最大业务字节数。
    pub max_bytes: u64,
    /// 最大执行秒数。
    pub max_seconds: u32,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Published {
    /// 不可变执行契约。
    pub contract: ExecutionContract,
    /// 平台发布的公开契约 SHA256 摘要。
    pub digest: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogPage {
    /// 本页本人获权执行契约。
    pub items: Vec<Published>,
    /// 当前服务器授权总数。
    pub total: u64,
    /// 当前页码。
    pub page: u32,
    /// 每页数量。
    pub page_size: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewOption {
    /// AUTH 返回的业务视角标识。
    pub view_id: u32,
    /// 七类 BOH 视角类别。
    pub view_type: ViewType,
    /// BOH 数据类型，可空。
    pub data_type: Option<i32>,
    /// 视角名称。
    pub view_name: String,
    /// 原系统对象标识。
    pub object_id: String,
    /// 原系统对象名称。
    pub object_name: String,
    /// 业务对象类型，可空。
    pub object_type: Option<String>,
    /// 门店编码，非门店可空。
    pub shop_code: Option<String>,
    /// 品牌标识，可空。
    pub brand_id: Option<i32>,
    /// 上级标识，可空。
    pub parent_id: Option<i32>,
    /// 目标是否支持此视角。
    pub supported: bool,
    /// 不支持时的准确原因。
    pub unsupported_reason: Option<String>,
    /// 最近视角在本次服务端合法候选中的提示，不自动选择。
    #[serde(default)]
    pub recently_selected: bool,
}
