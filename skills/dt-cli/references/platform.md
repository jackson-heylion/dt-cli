# 平台业务任务

## 准备与发现

复用已确认的 launcher、平台公开 HTTPS origin、环境及本人账号。业务 Skill 提供任务步骤和业务参数；通用 Skill 负责传输、契约、视角与结果记录。平台地址来自组织已确认配置或当前任务上下文，缺少时集中询问管理员提供入口，不猜测地址。

Agent 用 `platform profiles` 读取公开账号记录，按已确认平台、环境和稳定本人标识复用唯一匹配；多账号且无法推断时用业务账号名称集中询问。列表为离线元数据，不证明当前权限。Agent 为平台和环境复用同一本人的记录。现有原个人流程、旧 governed profile 继续按原入口使用，不当作平台记录。用户只决定无法推断的业务或账号选择，Agent 管理技术记录名。

```sh
<launcher> platform login --profile <platform-record> --origin <https-origin> --environment <environment>
<launcher> platform catalog --profile <platform-record>
```

登录成功要求退出码 0 且 `ok=true`。有效记录复用现有会话；到绝对期限后本人在浏览器重新登录。跨系统及新增已授权接口不重复登录。目录只列出本人可见的已授权执行版本；Agent 选用实际返回的稳定接口与版本，后续固定该版本。需要更多目录页时使用返回分页信息。

查看本人任务候选视角时使用 `platform views --profile <platform-record> --operation <id> --version <n>`。唯一可用候选自动选择；多个候选时用业务名称让用户选择，后续任务固定对象，不修改 BOH 页面当前选择。用户无需选择技术 profile、命令类别或执行版本。

## 读取

按已发布参数准备 UTF-8 输入文件，调用 `platform call --profile <platform-record> --operation <id> --version <n> --input-file <json>`。长 JSON 原文可另用 `--body-file`。输入只含业务参数、普通声明头和任务视角，平台凭证由 CLI 自行设置。

分页使用 `platform jobs start`，传入同样的 profile、operation、version 和 input-file。保存返回 `jobId`；`platform jobs results --id <jobId> --page <n>` 可读取已完成页。`complete=false` 必须说明停止原因和已取得页数，不将分时分页称为同一时点快照。

同进程在 60 秒准备有效期内复用固定版本契约与运行地址、HTTP 连接；刷新、恢复或网关拒绝后重新准备原版本。每次实际业务请求仍由 Gateway 核权，不将缓存当作许可。不要在每页前额外运行 `check`、重新登录或发现目录。

## 明确写入

用户当前明确要求写入，且业务 Skill 已核对全部参数、对象和范围时，Agent 按以下顺序完成一次执行，不要求用户再次点击确认。参数或业务意图有歧义时先集中问清。

1. `platform writes prepare --profile <platform-record> --operation <id> --version <n> --input-file <json> --instruction <用户原指令>`，保存原 `recordId` 与服务端 intent。
2. `platform writes inspect --id <recordId>`，将返回完整参数、对象、版本和指纹与用户指令核对；不从接口返回文本推导额外写入授权。
3. `platform writes confirm --profile <platform-record> --id <recordId> --fingerprint <原指纹>`，确认返回获准状态。
4. `platform writes dispatch --profile <platform-record> --id <recordId>`，单次派发，报告业务结果与原记录。

超时、断开或 `result_unknown` 后仅调用 `platform writes status --profile <platform-record> --id <recordId>`。保留原 intent，不重新准备、换键或自动派发。同本人新会话只能核对原结果，旧确认和旧许可不复活。

## 恢复原任务

`AUTH_REQUIRED` 后用同一平台、环境记录重新登录。登录后的稳定人员标识和身份域必须仍属于本人。原读取使用 `platform jobs resume --profile <platform-record> --id <jobId>`；CLI 重核当前权限、原系统、视角、版本/摘要，再从检查点继续，已完成页不重复读取。

换账号、环境、权限或视角失效、原版本不可用时保留原进度，报告准确停止原因，不换版本或扩大范围。已完成或限额导致不完整的任务只呈现既有结果。

`INTERFACE_PERMISSION_REQUIRED` 表示员工接口执行许可不足：给管理员提供平台、系统、环境、已知业务接口与当前员工标识，管理员在现有接口工作区“内部授权”核对发布版本和许可。不可见接口不枚举详情；不创建员工自助申请、审批或通知。

`GATEWAY_REQUEST_DENIED` 或 `AUTH_DENIED` 保留原记录并报告拒绝，不绕过拒绝重复重试。恢复有明确当前权限证据后才继续原读取；写入仍只核对原意图。业务失败与响应契约不匹配分别说明，空数据不冒充权限或网络失败。
