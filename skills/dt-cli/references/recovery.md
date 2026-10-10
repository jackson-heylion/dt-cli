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
| 安全存储不可用、凭证保存失败 | 保留原 profile，按[本地存储故障](#本地存储故障)运行 CLI 诊断；根据失败步骤处理，不反复登录。 |
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

沙盒拒绝、重复弹窗或已有 EPERM 证据时先按[沙盒执行](sandbox.md)核对宿主权限。尚无该执行环境的诊断结果时，使用已确认的绝对 launcher 执行一次 `doctor --storage`。它只用无敏感内容的专用临时文件检查 credentials、profile 和业务目录的创建、写入、同步、原子替换、回读及删除；不读取员工凭证，不扫描或清理其他文件。默认 `doctor` 仍不探测存储。

按 `error.details.area`、`stage`、`reason`、`osCode` 定位：

- validation 的权限或所有者错误：核对 macOS/Linux 当前 UID、目录 0700、文件 0600，Windows 当前用户专属 ACL；只修复证据指向的对象。
- create、write、atomic_replace、sync 或 delete 被拒绝：交客户端或终端支持核对该执行环境的文件操作策略。`Operation not permitted` 本身不能确定是沙箱、系统 ACL 或企业策略。
- 诊断清理失败：记录返回的 `cleanupPath` 或 `cleanup.path`，由用户或客户端支持在允许的终端确认诊断进程已结束后处理该准确文件；不追加 shell 删除命令。禁止用 `.tmp*` 通配清理；现有临时条目不会使整个凭证目录被拒绝。

凭证文件名是绑定 key 的 SHA-256，凭证正文不含 `credentialKey`；这些信息无需通过读取凭证验证。`auth status` 是本地状态，不证明在线授权或存储写入可用。出现登录失败的 remoteRevocation/newCredentialCleanup 表示处理过新授权，不能解释成“尚未发生网络调用”。

修复后只重试原操作一次。仍失败时保留原账号与凭证，交付诊断结果、实际执行环境及准确失败步骤，不搬移真实凭证到临时目录。安全检查只能报告已验证的来源和校验结果，不能根据域名或脚本摘要宣称所有行为都安全。
