//! 命令解析产生的具名请求；业务层不接收解析器或泛化字典。
#[derive(Clone, Default)]
pub(crate) struct Target {
    /// 明确接口标识；使用别名时为空。
    pub operation: String,
    /// 明确业务别名；使用接口标识时为空。
    pub alias: String,
    /// 明确系统范围，可空。
    pub system: String,
    /// 明确固定执行版本，未指定时为空。
    pub version: Option<u64>,
}
#[derive(Clone, Default)]
pub(crate) struct Files {
    /// 具名调用输入文件路径。
    pub input: String,
    /// 原业务正文文件，可空。
    pub body: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Login {
    /// 本次运行环境；默认正式环境。
    pub environment: String,
    /// 开发夹具地址；发行构建仍拒绝任意地址。
    pub origin: Option<String>,
    /// 本次业务系统，不改变凭证用途。
    pub system: String,
    /// 是否由员工手动打开本次登录链接。
    pub no_browser: bool,
}
#[derive(Clone)]
pub(crate) struct Search {
    /// 系统筛选条件。
    pub system: String,
    /// 接口名称筛选条件。
    pub name: String,
    /// HTTP 方法筛选条件。
    pub method: String,
    /// 原接口路径筛选条件。
    pub path: String,
    /// 服务端页码，从一开始。
    pub page: u32,
    /// 服务端单页条数。
    pub page_size: u32,
}
#[derive(Clone, Default)]
pub(crate) struct Template {
    /// 保存模板的明确路径，未指定时仅预览。
    pub output: Option<String>,
    /// 被替换文件的明确摘要，可空。
    pub overwrite_digest: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Diagnostic {
    /// 已选定的原接口。
    pub target: Target,
    /// 是否明确执行一次读取探测。
    pub probe: bool,
    /// 已声明的探测参数。
    pub files: Files,
    /// 用户明确的数据范围。
    pub scope: String,
}
#[derive(Clone)]
pub(crate) struct Call {
    /// 已明确的接口或别名。
    pub target: Target,
    /// 调用输入文件，未提供时采用路径与查询串。
    pub files: Files,
    /// 原业务路径。
    pub path: String,
    /// 原业务查询串。
    pub query: String,
    /// 明确任务视角编号，可空。
    pub view: Option<u32>,
    /// 视角对象编号，可空。
    pub object: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Record {
    /// 已验证动作名称。
    pub action: &'static str,
    /// 明确本人记录，可空时使用对应用途上下文。
    pub profile: Option<String>,
    /// 原本机记录编号。
    pub id: String,
    /// 本机记录页码；结果页可为零。
    pub page: u32,
    /// 新任务固定接口。
    pub target: Target,
    /// 新任务原参数文件。
    pub files: Files,
    /// 用户明确业务指令；只用于准备写入。
    pub instruction: String,
    /// 原确认指纹；只用于确认写入。
    pub fingerprint: String,
}
#[derive(Clone)]
pub(crate) struct Context {
    /// use、show 或 clear。
    pub action: &'static str,
    /// 对应用途的明确本人记录，可空。
    pub profile: Option<String>,
    /// 明确环境，设置时必填。
    pub environment: String,
    /// management 为独立管理用途，business 为业务用途。
    pub purpose: &'static str,
}
#[derive(Clone)]
pub(crate) struct Management {
    /// 已验证管理动作。
    pub action: &'static str,
    /// 明确管理本人记录，可空时使用管理上下文。
    pub profile: Option<String>,
    /// 管理登录参数，非登录动作为空。
    pub login: Option<Login>,
    /// 原对象或计划编号，可空。
    pub id: Option<String>,
    /// 服务端数据库页码，可空。
    pub page: Option<u32>,
    /// 系统环境范围，可空。
    pub system: Option<String>,
    /// 原接入清单路径。
    pub manifest: String,
    /// 原计划摘要。
    pub digest: String,
    /// 原执行请求号，可空。
    pub request_id: Option<String>,
    /// 明确继续核对原执行租约。
    pub retry: bool,
}
#[derive(Clone)]
pub(crate) enum Action {
    Profiles,
    Login(Login),
    Check,
    Sessions(u32),
    Revoke(String),
    Logout,
    Catalog(u32),
    Views(Target),
    Call(Call),
    Search(Search),
    Resolve(Target),
    Describe(Target),
    Template(Target, Template),
    Diagnose(Diagnostic),
    Jobs(Record),
    Writes(Record),
    Context(Context),
    Manage(Management),
}
#[derive(Clone)]
pub(crate) struct Platform {
    /// 当前动作的明确本人记录。
    pub profile: Option<String>,
    /// 具名业务动作及其请求。
    pub action: Action,
}
#[derive(Clone)]
pub(crate) struct Native {
    /// 是否安装新的独立目录。
    pub install: bool,
    /// 是否只检查安装或发行。
    pub check: bool,
    /// 是否回退当前目录上一版本。
    pub rollback: bool,
    /// 是否清理当前安装目录冗余版本。
    pub prune: bool,
    /// 是否使用独立候选安装目录。
    pub candidate: bool,
    /// 是否从固定可信源更新。
    pub online: bool,
    /// 是否允许受限可信缓存。
    pub cached: bool,
    /// 本地程序包路径，可空。
    pub package: Option<String>,
    /// 明确安装目录，可空时使用固定默认目录。
    pub directory: Option<String>,
    /// 用户核对的程序包摘要，可空。
    pub sha256: Option<String>,
    /// 平台签名者，可空。
    pub signer: Option<String>,
    /// 原任务要求的最低版本，可空。
    pub minimum_version: Option<String>,
}
#[derive(Clone)]
pub(crate) struct Skill {
    /// 明确技能名称。
    pub name: String,
    /// 明确安装目录。
    pub directory: String,
    /// 是否只检查本地版本。
    pub check: bool,
    /// 是否回退本目录上一版本。
    pub rollback: bool,
}
