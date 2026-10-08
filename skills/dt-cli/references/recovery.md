# 错误恢复

| 状态 | 动作 |
| --- | --- |
| `BOOTSTRAP_FAILED`、`DISTRIBUTION_NOT_CONFIGURED`、`DISTRIBUTION_UNAVAILABLE` | 核对随 Skill 发布的固定 HTTPS 入口和安装结果；兼容旧版的更新检查失败可继续使用，缺少可用程序或最低版本不满足时保留任务并报告阻塞。 |
| `DISTRIBUTION_INVALID`、`DISTRIBUTION_ROLLBACK_BLOCKED` | 交维护者核对发布序号、不可变版本、摘要和 CDN；保留当前程序，不改下载域名或接受不同摘要。 |
| `AUTH_REQUIRED`、`AUTHORIZATION_EXPIRED`、`AUTHORIZATION_REVOKED` | 核对当前 profile；Agent 调用该 profile 的登录命令，由本人完成浏览器登录。重新登录不恢复管理员授予的接口权限。 |
| `GRANT_EXPIRED`、`SCOPE_DENIED` | 报告当前接口、系统与环境，由管理员核对权限；不更换员工或扩大范围。 |
| `CONTRACT_CHANGED` | 同步目录，明确选择新合同，由管理员授予新版本权限；已有 intent 保留并先查询，不静默重新准备。 |
| `CLIENT_UPGRADE_REQUIRED` | 从组织可信发行入口升级，验证版本、ZIP 摘要和构建来源；不下载任意同名程序。 |
| 安全存储不可用、凭证保存失败 | 保留原 profile；修复本地 credentials 目录权限（macOS/Linux 0700/0600，Windows 当前用户专属 ACL），不把 token 输出到会话。 |
| 部分完成或退出码 7 | 保留 `data` 和完整性 `meta`，报告实际取得数量与停止原因；恢复策略限定在用户原查询范围。 |
| `RESOURCE_UNAVAILABLE`、`JUMP_UNAVAILABLE` | 消息不可访问或来源无支持入口，报告原因；不猜测跳转地址。 |
| `SIDE_EFFECT_UNKNOWN`、`ALREADY_DISPATCHED` | 查询原 intentId，结果待核对时人工核对；不再调用 invoke，不重新准备同一内容。 |
| 退出的 `remoteRevocation=unknown` | 本地清理与远端撤销分别报告；通过 IAM 授权管理页核对远端状态。 |

`auth status` 和默认 `doctor` 是离线结果。`auth check`、`whoami` 或 `doctor --online` 是在线探针；诊断不暗中遍历业务数据。错误信息、版本、环境和脱敏 trace 可用于定位，授权码、token、密码、跳转凭据和浏览器 cookie 不进入报告。

授权页没有可选接口时，先按[管理员赋权入口](governed.md#管理员赋权入口)核对发布状态、员工标识及有效期。管理记录的 `ACTIVE` 状态还需结合到期时间判断；空列表不表示业务数据为零，也不通过反复登录恢复赋权。

0.4.x 优先解释 `meta.actions[0]` 的 actor、reason 和 argv/可信 URL。使用 argv 数组；按用户已有业务指令完成必要安装和登录准备。管理员操作与业务写入须符合用户明确范围，`requiresInteraction=true` 的本人步骤需本人参与。用户取消或登录超时停止本次授权步骤，不循环弹窗。含 runId 的工作包恢复见[工作包](tasks.md)；同一 runId 恢复只查询，授权恢复后也不重新准备或派发。
