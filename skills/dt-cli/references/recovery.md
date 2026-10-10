# 错误恢复

| 状态 | 动作 |
| --- | --- |
| `BOOTSTRAP_FAILED`、`DISTRIBUTION_NOT_CONFIGURED`、`DISTRIBUTION_UNAVAILABLE` | 核对随 Skill 发布的固定 HTTPS 入口和安装结果；兼容旧版的更新检查失败可继续使用，缺少可用程序或最低版本不满足时保留任务并报告阻塞。 |
| `DISTRIBUTION_INVALID`、`DISTRIBUTION_ROLLBACK_BLOCKED` | 交维护者核对发布序号、不可变版本、摘要和 CDN；保留当前程序，不改下载域名或接受不同摘要。 |
| `PROFILE_NOT_CONFIGURED`、`PROFILE_REQUIRED`、`PROFILE_SELECTION_MISMATCH` | 按[首次使用](first-use.md)选择匹配系统和环境的账号；没有匹配账号时由 Agent 发起浏览器登录。 |
| `INTERACTION_REQUIRED` | 已授权的登录使用 `auth login ... --interaction browser`，保留原账号绑定，由本人完成浏览器验证。 |
| `AUTH_REQUIRED`、`AUTHORIZATION_EXPIRED`、`AUTHORIZATION_REVOKED` | 核对当前 profile；Agent 调用该 profile 的登录命令，由本人完成浏览器登录。重新登录不恢复管理员授予的接口权限。 |
| `GRANT_EXPIRED`、`SCOPE_DENIED` | 报告原业务接口及精确版本、系统、环境、当前主体和脱敏 traceId，由管理员核对授权与业务数据范围；保持原 profile，不更换员工或扩大范围。管理员确认恢复后重试原读取一次，仍拒绝即交付本次证据，不重复登录、改分页参数或提交其他任务探测。 |
| `CONTRACT_CHANGED` | 同步目录，明确选择新合同，由管理员授予新版本权限；已有 intent 保留并先查询，不静默重新准备。 |
| `CLIENT_UPGRADE_REQUIRED` | 从组织可信发行入口升级，验证版本、ZIP 摘要和构建来源；不下载任意同名程序。 |
| `CREDENTIAL_STORE_UNAVAILABLE`、`LOCAL_STATE_UNAVAILABLE`、文件访问拒绝 | 保留原 profile，运行一次 `doctor --storage`，按失败步骤恢复本地访问后继续原任务；不读取或搬移凭证。 |
| `RATE_LIMITED`（读取） | 新 CLI 对 api call、jobs status/result 的明确 429 按 `retryAfterSeconds` 自动最多重试两次，总时间 60 秒；写入及 jobs submit 不自动重发。用尽预算后按返回时间继续原读取。持续限流时检查是否有同主体的后台读取共享预算；保留原 jobId 等待，不新增批量任务或修改限流记录。 |
| 部分完成或退出码 7 | 保留 `data` 和完整性 `meta`，报告实际取得数量与停止原因；恢复策略限定在用户原查询范围。 |
| `RESOURCE_UNAVAILABLE`、`JUMP_UNAVAILABLE` | 消息不可访问或来源无支持入口，报告原因；不猜测跳转地址。 |
| `SIDE_EFFECT_UNKNOWN`、`ALREADY_DISPATCHED` | 查询原 intentId，结果待核对时人工核对；不再调用 invoke，不重新准备同一内容。 |
| 退出的 `remoteRevocation=unknown` | 本地清理与远端撤销分别报告；通过 IAM 授权管理页核对远端状态。 |

`auth status` 和默认 `doctor` 是离线结果。`auth check`、`whoami` 或 `doctor --online` 是在线探针；用户要求核对身份、诊断，或实际错误需要区分原因时，选一个相关命令。普通业务查询直接调用接口，由返回判断恢复分支。错误信息、版本、环境和脱敏 trace 可用于定位，授权码、token、密码、跳转凭据和浏览器 cookie 不进入报告。

授权页没有可选接口时，先按[管理员赋权入口](governed.md#管理员赋权入口)核对发布状态、员工标识及有效期。管理记录的 `ACTIVE` 状态还需结合到期时间判断；空列表不表示业务数据为零，也不通过反复登录恢复赋权。

供应链精确 Grant 已确认正确仍返回 `SCOPE_DENIED` 时，见[供应链配送中心选择](governed.md#供应链配送中心选择与少量读取)，核对选定中心的权限及新旧合同适配方式。

0.4.x 优先解释 `meta.actions[0]` 的 actor、reason 和 argv/可信 URL。使用 argv 数组；按用户已有业务指令完成必要安装和登录准备。管理员操作与业务写入须符合用户明确范围，`requiresInteraction=true` 的本人步骤需本人参与。用户取消或登录超时停止本次授权步骤，不循环弹窗。含 runId 的工作包恢复见[工作包](tasks.md)；同一 runId 恢复只查询，授权恢复后也不重新准备或派发。

## 目录与参数错误

先核对 `catalogSource`、profile、systemId 和 environment。`bundled-cli` 的缺项不能证明业务接口缺失。CLI、Skill、目录和接口版本分别独立，按兼容字段判断。

`INVALID_ARGUMENT`：检查原接口、原 profile、原版本的 Schema，使用实际字段修正参数。
`UNKNOWN_OPERATION`：选择正确业务账号，目录缺失或原接口不在目录时同步一次，再查看同一接口。仍缺少时交管理员核对发布和授权。恢复命令保持已确认 launcher 的绝对路径。

## 本地存储故障

`CREDENTIAL_STORE_UNAVAILABLE` 或 `LOCAL_STATE_UNAVAILABLE` 时，保留原 profile，运行一次 `doctor --storage`，按返回的 `area`、`stage`、`reason`、`osCode` 排查并恢复本地访问，再继续原任务。诊断只使用 CLI 自己创建的临时文件，不扫描或清理其他文件。

macOS 或 Windows WorkBuddy 中的 `atomic_replace` / `permission_denied`：用支持此入口的 launcher 运行 `doctor --workbuddy`，查看 dt-cli 数据目录是否已配置。缺少配置时，向用户说明“需要允许 WorkBuddy 读写、重命名和删除 dt-cli 的本地数据目录”，由用户在本机终端执行修复命令，或在 WorkBuddy 文件权限设置中添加返回的 `allowDirectories`。Windows 返回账号与安装数据两个目录，均需配置。旧 CLI 没有此命令时使用客户端文件权限入口。

```sh
"<launcher>" doctor --workbuddy --fix
```

```powershell
& "<launcher.exe>" doctor --workbuddy --fix
```

此命令写入 `settingsKey=sandbox.extraAllowWrite`；`allowDirectory` 是目录路径，`configured=true` 仅表示配置文件包含该目录。保存后请用户完全退出并重新打开 WorkBuddy，在新会话直接执行 `<launcher> doctor --storage` 并检查 `ok=true`，通过后继续原登录或查询。诊断命令保留原始退出码与 JSON，不接 `tail` 或仅提取 `data` 的过滤管道。经宿主授权改用普通子进程重跑的成功，不能证明原沙盒规则已加载；一次通过一次失败时先核对执行方式与配置加载。Python、shell 或普通终端的成功也不能单独证明原生进程权限或二进制签名原因。

不要读取、搬移或手工修改凭证。若诊断的清理失败，记录返回的准确路径，由用户或支持人员确认后处理；不要通配清理临时文件。
