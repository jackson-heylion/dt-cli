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

`unsafe_acl_or_owner` 表示 Windows 文件所有者或访问控制未通过安全校验。按具体失败对象与运行方式核对原生进程用户。需要修复时，由现场先备份该对象权限，再按 CLI 对当前用户的权限要求定向调整，保留已有账号与凭证。用原启动器复跑 `doctor --storage` 与 `profiles`，记录修复前后的退出码与结果。

macOS 或 Windows WorkBuddy 中的 `atomic_replace` / `permission_denied`：用支持此入口的 launcher 运行 `doctor --workbuddy`，查看 dt-cli 数据目录是否已配置。缺少配置时，向用户说明“需要允许 WorkBuddy 读写、重命名和删除 dt-cli 的本地数据目录”，由用户在本机终端执行修复命令，或在 WorkBuddy 文件权限设置中添加返回的 `allowDirectories`。Windows 返回账号与安装数据两个目录，均需配置。

`WORKBUDDY_CONFIG_UNAVAILABLE` 的 `configurationVerified=false` 表示目录许可状态未知。按 `area` 区分 `workbuddy_directory` 和 `workbuddy_settings`。遇到 `reparse_point` 使用 WorkBuddy 自身的设置入口，`open_failed` 核对实际路径及访问，`unsafe_permissions` 核对对象所有者和修改权限。相同原因未消除时停止重复 `--fix`。目录写权限不能证明浏览器窗口已显示或回调端口可达。

实际存储检查和业务均通过时，目录许可未登记或设置目录是 Junction 只需在排障记录中说明。按实际文件权限故障处理，或按本人明确要求检查新许可。

```sh
"<launcher>" doctor --workbuddy --fix
```

```powershell
& "<launcher.exe>" doctor --workbuddy --fix
```

此命令将数据目录加入 `sandbox.extraAllowWrite`。诊断中的 `configured` 反映是否已登记，`storageVerified=false` 表示此命令尚未验证实际权限。验收新保存的许可时，请本人完全退出并重新打开 WorkBuddy，在新会话的原工具环境用同一启动器运行 `doctor --storage`，需要核对存量账号时再运行 `profiles`。普通终端或经宿主授权的普通子进程结果仅作对照。排障保留原始退出码和 JSON。

不要读取、搬移或手工修改凭证。若诊断的清理失败，记录返回的准确路径，由用户或支持人员确认后处理；不要通配清理临时文件。
