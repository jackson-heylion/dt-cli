# 平台恢复

先核对最终信封的 error.code、category、retryable 及 meta.recovery。retryable 仅表示原请求安全重发；故障类别不提供写入重发许可。恢复动作使用 argv 参数数组，并保留原本人、环境、用途和记录。

| 当前结果 | 处理 |
| --- | --- |
| AUTH_REQUIRED | 沿原用途与环境完成本人登录；读取恢复原 jobId，写入只查询原 recordId。 |
| INTERFACE_PERMISSION_REQUIRED / AUTH_DENIED / DATA_SCOPE_DENIED | 核对精确接口许可与原对象范围，不通过反复登录或换对象恢复权限。 |
| EXECUTION_CONTRACT_MISMATCH / VIEW_NOT_ALLOWED | 保留原结果，核对原版本、契约和合法视角后再继续。 |
| NETWORK_ERROR / RATE_LIMITED / DEPENDENCY_UNAVAILABLE | 读取按原上下文和剩余预算恢复；写入派发尝试已落盘时只核对原记录。 |
| LOGOUT_PARTIAL（退出码7） | 分别报告 remoteRevocation、credentialCleanup、profileCleanup、contextCleanup。远端未知不能报完整撤销。 |
| INVALID_ARGUMENT | 使用根 help 或对应命令 --help，核对声明参数；不回显凭证或完整业务输入。 |

写入结果未知使用 writes status --profile <原账号> --id <原recordId>。已记录派发尝试不换请求号、不另建意图、不自动再次派发。新会话仅可核对旧结果，不能恢复旧确认。

## 本地存储故障

`CREDENTIAL_STORE_UNAVAILABLE` 或 `LOCAL_STATE_UNAVAILABLE` 时，保留原本地账号记录。按 `area`、`stage`、`reason` 和 `osCode` 定位。`profile_directory` 指账号目录，`profile_record` 指单条记录，`profileRecord` 有值时给出该记录名。已有明确对象时按对象处理。`doctor --storage` 只检查目录和自己创建的临时探针，`existingProfilesChecked=false` 时用 `profiles` 核对存量账号记录。

`unsafe_acl_or_owner` 表示 Windows 文件所有者或访问控制未通过安全校验，和 `permission_denied` 分开处理。记录具体失败对象与运行方式，核对原生进程用户及该对象的 ACL；仅凭沙箱与宿主表现不同不能确定原因。保留已有账号与凭证，不通过扩大 ACL 绕过校验；诊断记录确认属于本次且无人使用的临时对象才可精确处理。

macOS 或 Windows WorkBuddy 中的 `atomic_replace` / `permission_denied`：用支持此入口的 launcher 运行 `doctor --workbuddy`，查看 dt-cli 数据目录是否已配置。缺少配置时，向用户说明“需要允许 WorkBuddy 读写、重命名和删除 dt-cli 的本地数据目录”，由用户在本机终端执行修复命令，或在 WorkBuddy 文件权限设置中添加返回的 `allowDirectories`。Windows 返回账号与安装数据两个目录，均需配置。

`WORKBUDDY_CONFIG_UNAVAILABLE` 的 `configurationVerified=false` 表示目录许可状态未知。按 `area` 区分 `workbuddy_directory` 和 `workbuddy_settings`。遇到 `reparse_point` 使用 WorkBuddy 自身的设置入口，`open_failed` 核对实际路径及访问，`unsafe_permissions` 核对对象所有者和修改权限。相同原因未消除时停止重复 `--fix`。目录写权限不能证明浏览器窗口已显示或回调端口可达。

```sh
"<launcher>" doctor --workbuddy --fix
```

```powershell
& "<launcher.exe>" doctor --workbuddy --fix
```

此命令写入 `settingsKey=sandbox.extraAllowWrite`；`allowDirectory` 是目录路径，`configured=true` 仅表示配置文件包含该目录。保存后完全退出并重新打开 WorkBuddy，在新会话用同一启动器运行 `doctor --storage` 和 `profiles`。两者通过后继续原登录或查询。保留原始退出码与 JSON，不用只提取 `data` 的过滤管道。普通终端或经宿主授权的普通子进程结果仅用于对照，实际沙盒权限以原工具环境复验为准。

不要读取、搬移或手工修改凭证。若诊断的清理失败，记录返回的准确路径，由用户或支持人员确认后处理；不要通配清理临时文件。
