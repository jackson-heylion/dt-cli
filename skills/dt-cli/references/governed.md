# 受控业务

0.4.x 的登记工作包与 runId 恢复见[工作包](tasks.md)；本页保留原业务命令路径。

每个系统、环境、员工使用独立 profile，不能复用个人流程授权。已有 launcher、绑定、接口版本和参数时直接读取。以下命令按缺失信息选用：登录仅用于缺少有效登录，catalog sync 用于目录缺失或合同变化，discover 与 schema 用于未知能力或参数；它们不构成每次查询的前置流水线。

```sh
dt-cli auth login --profile <profile> --environment <environment> --system <system> --interaction browser
dt-cli catalog sync --profile <profile>
dt-cli discover --profile <profile> --query <关键词>
dt-cli schema <operation-id> --profile <profile> --version <version>
dt-cli api call <read-operation-id> --profile <profile> --version <version> --params-file request.json
```

目录里同一操作有多个版本时明确选择版本。首次确定调用方式时核对 `effect`、输入 Schema 和确认渠道；已确认的信息直接复用。目录的 `consented` 只说明缓存时状态，服务端每次执行仍检查授权。合同变化时同步并重新选择，不静默切换版本。

后台读取使用 `jobs submit`，保留 jobId，再用 `jobs wait <jobId> --profile <profile> --timeout 30` 有限等待。当前 CLI 的 wait 在成功后自动下载完整结果；已返回完整 items 时直接整理，只有用户要求或尚未取得结果时再调用 jobs result。等待超时继续观察原 job；失败信息未带 failureCode 时查一次 jobs status。也可用 status → succeeded → result，不固定 sleep 或重复提交。取消用 jobs cancel；排队不等于完成。

少量查询使用合同支持的分页或范围字段。展示前几行只是展示限制，不代表实际读取量受限。

## 管理员赋权入口

接口授权列表为空、管理员赋权到期或范围不足时，请用户本人或授权管理员打开当前环境 IAM 的「接口管理 → 权限与岗位」，核对权限组中的精确接口版本、有效期和授权对象。员工可填写 IAM 用户 ID、用户编码 `code` 或账号，多个对象用逗号分隔；歧义标识可用 `id:`、`code:`、`account:` 前缀明确指定。岗位角色仍填写角色 ID。

「全员」动态覆盖当前及后续新增的有效在职员工，停用、离职、临时及非员工账号不适用。全员赋权保留精确接口合同、到期撤销和业务数据范围校验。管理员恢复原接口的授权或数据范围后，复用原员工 profile 直接读取；新增接口不在缓存目录中或合同变化时才 catalog sync。仅在缺少登录或登录到期时重新登录。重新登录本身不会增加管理员权限。
