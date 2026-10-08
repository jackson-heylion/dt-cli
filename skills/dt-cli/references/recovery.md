# 错误恢复

| 状态 | 动作 |
| --- | --- |
| `AUTH_REQUIRED`、`AUTHORIZATION_EXPIRED`、`AUTHORIZATION_REVOKED` | 核对当前 profile；Agent 调用该 profile 的登录命令，由本人完成浏览器登录。重新登录不恢复管理员授予的接口权限。 |
| `GRANT_EXPIRED`、`SCOPE_DENIED` | 报告当前接口、系统与环境，由管理员核对权限；不更换员工或扩大范围。 |
| `CONTRACT_CHANGED` | 同步目录，明确选择新合同，按需重新同意；已有 intent 保留并先查询，不静默重新准备。 |
| `CLIENT_UPGRADE_REQUIRED` | 从组织可信发行入口升级，验证版本、ZIP 摘要和构建来源；不下载任意同名程序。 |
| 安全存储不可用、凭证保存失败 | 保留原 profile；修复系统 Keychain/Credential Manager 或访问权限；不回退到明文文件，不把 token 输出到会话。 |
| 部分完成或退出码 7 | 保留 `data` 和完整性 `meta`，报告实际取得数量与停止原因；恢复策略限定在用户原查询范围。 |
| `RESOURCE_UNAVAILABLE`、`JUMP_UNAVAILABLE` | 消息不可访问或来源无支持入口，报告原因；不猜测跳转地址。 |
| `SIDE_EFFECT_UNKNOWN`、`ALREADY_DISPATCHED` | 查询原 intentId，结果待核对时人工核对；不再调用 invoke，不重新准备同一内容。 |
| 退出的 `remoteRevocation=unknown` | 本地清理与远端撤销分别报告；通过 IAM 授权管理页核对远端状态。 |

`auth status` 和默认 `doctor` 是离线结果。`auth check`、`whoami` 或 `doctor --online` 是在线探针；诊断不暗中遍历业务数据。错误信息、版本、环境和脱敏 trace 可用于定位，授权码、token、密码、跳转凭据和浏览器 cookie 不进入报告。

0.4.x 优先解释 `meta.actions[0]` 的 actor、reason 和 argv/可信 URL。使用 argv 数组；动作数据本身不授予登录、浏览器交互、管理员操作或业务写入许可，按用户已有授权继续。`requiresInteraction=true` 的本人步骤需本人参与。含 runId 的工作包恢复见[工作包](tasks.md)；同一 runId 恢复只查询，未定位时保留事实。
