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

少量查询优先使用合同支持的分页或范围字段。batch 合同没有分页或数量限制时，保留一个真实品项筛选，说明后台会读取该品项在可见门店的完整配置，展示前几行只是展示限制；用户要求限制实际读取量时选择支持分页的单店接口。

## 管理员赋权入口

接口授权列表为空、管理员赋权到期或范围不足时，请用户本人或授权管理员打开当前环境 IAM 的「接口管理 → 权限与岗位」，核对权限组中的精确接口版本、有效期和授权对象。员工可填写 IAM 用户 ID、用户编码 `code` 或账号，多个对象用逗号分隔；歧义标识可用 `id:`、`code:`、`account:` 前缀明确指定。岗位角色仍填写角色 ID。

「全员」动态覆盖当前及后续新增的有效在职员工，停用、离职、临时及非员工账号不适用。全员赋权保留精确接口合同、到期撤销和业务数据范围校验。管理员恢复原接口的授权或数据范围后，复用原员工 profile 直接读取；新增接口不在缓存目录中或合同变化时才 catalog sync。仅在缺少登录或登录到期时重新登录。重新登录本身不会增加管理员权限。

### 供应链配送中心选择与少量读取

已发布的配送中心参数合同支持 `deliveryCenterId`，由 IAM/AUTH 验证本人有权使用该中心，独立临时凭证不读取或修改 BOH 当前视角。已有用户指定的中心直接使用；缺少中心时调用 `supply-chain-server.delivery-centers.list`，可按 `name` 或 `deliveryCenterId` 定位真实对象。只有一个候选自动选择，多个候选复用当前用户选择或 profile 默认值，无法唯一确定才询问。空列表说明没有当前可用中心，交管理员核对。

```sh
dt-cli api call supply-chain-server.delivery-centers.list --profile <profile> --version <已发布版本> --params '{"name":"<公司名称>"}'
dt-cli profiles delivery-center --profile <profile> --id <真实配送中心ID>
dt-cli api call supply-chain-server.shops.list --profile <profile> --version <配送中心参数合同版本> --params '{"pageNum":1,"pageSize":5}'
dt-cli api call supply-chain-server.order-config.read --profile <profile> --version <配送中心参数合同版本> --params '{"shopCode":"<真实门店码>","itemCode":"<真实品项码>","pageNum":1,"pageSize":5}'
```

profile 默认中心仅适用于合同已声明 deliveryCenterId 的 api call / jobs submit；显式参数优先。保存偏好不代表授权，服务端每次仍校验。tasks run order-config.compare 使用显式 deliveryCenterId，以便将本次中心固定到恢复摘要。少量查询走单店分页；明确要跨门店完整配置时才 batch.read。新 job 将中心保存在原参数中，执行和结果下载都使用同一中心。

旧合同没有此参数时仍沿用 BOH 已选配送中心。授权正确仍报 SCOPE_DENIED 时，请维护者沿 traceId / invocationId 核对 AUTH 临时凭证签发；queued 后失败不能直接归因于门店范围。新合同未发布或未获 Grant 时说明服务端准备缺失，不猜版本或借用旧合同绕过。

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
