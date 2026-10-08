# 受控业务

0.4.x 的登记工作包与 runId 恢复见[工作包](tasks.md)；本页保留原业务命令路径。

每个系统、环境、员工使用独立 profile，不能复用个人流程授权。首次登录由 Agent 调用：

```sh
dt-cli auth login --profile <profile> --environment <environment> --system <system>
dt-cli catalog sync --profile <profile>
dt-cli discover --profile <profile> --query <关键词>
dt-cli schema <operation-id> --profile <profile> --version <version>
dt-cli api call <read-operation-id> --profile <profile> --version <version> --params-file request.json
```

目录里同一操作有多个版本时明确选择版本。核对 `effect`、输入 Schema 和确认渠道；目录的 `consented` 只说明缓存时状态，服务端每次执行仍检查授权。合同变化时同步并重新选择，不静默切换版本。

后台读取使用 `jobs submit`，保留 jobId；通过 `jobs status` 查询，只有 `succeeded` 且完整时调用 `jobs result`。取消用 `jobs cancel`。不把排队或部分页当成完成。

## 管理员赋权入口

接口授权列表为空、管理员赋权到期或范围不足时，请用户本人或授权管理员打开当前环境 IAM 的「接口管理 → 权限与岗位」，核对权限组中的精确接口版本、有效期和授权对象。员工可填写 IAM 用户 ID、用户编码 `code` 或账号，多个对象用逗号分隔；歧义标识可用 `id:`、`code:`、`account:` 前缀明确指定。岗位角色仍填写角色 ID。

「全员」动态覆盖当前及后续新增的有效在职员工，停用、离职、临时及非员工账号不适用。全员赋权保留精确接口合同、到期撤销和业务数据范围校验。管理员完成赋权后，复用原员工 profile 继续查询；新增后台权限先 catalog sync；仅在缺少登录或登录到期时重新登录。重新登录本身不会增加管理员权限。

## 点赞：后台赋权 → 准备 → 单次派发

从在线候选人和类型接口取得真实 personId、类型 ID、名称和行为，读取当前 Schema。候选人 `eligibility=unchecked`，资格由业务系统在提交时判断。用户指定对象不唯一时确认对象；用户仅要求起草时交付草稿。

用户明确要求发送后，用文件工具写入 `request.json`，字段遵循 Schema：`beLikedPersonId`、`likeTypeId`、`likeTypeName`、`likeBehaviorContent`、`specificDeeds`。不添加操作者 ID。

选用 `confirmation.channel=backend-grant`、最低 CLI 版本不高于当前版本的已发布合同。旧 `iam-browser` 合同需维护者发布新版本、授予后台权限；不要借用其摘要进行 CLI 授权。

```sh
dt-cli intents prepare hrmp.like.send --profile <profile> --version <version> --params-file request.json
dt-cli intents invoke <intent-id> --profile <profile>
```

两个命令顺序执行。保存 prepare 的 intentId、argumentsDigest、contractDigest、版本和幂等键；核对与当前业务任务的参数一致。后台有效 Grant 使 prepare 返回 approved/none，随后 invoke 一次；不再需要用户逐项同意或调用 authorize。旧 agent-cli 合同仅兼容既有流程，未上线新合同不能假报已经免授权。

只在 state=succeeded、sideEffect=confirmed 且 ok=true 时报告发送成功。后台赋权撤销、到期、员工停用仍阻止派发。参数缺失或对象不唯一时先澄清；完整且任务范围明确时直接执行，无需额外问允许。

超时、断连或取消调用可能已经派发。保存原 intentId，执行 `dt-cli intents status <intent-id> --profile <profile>`；`unknown` 时由目标系统核对，必要时使用返回的人工核对入口。取消未派发的意图用 `intents cancel`。不要换幂等键重发。`--wait` 仅等待服务端记录的授权，不替代 authorize，也不打开点赞确认页。
