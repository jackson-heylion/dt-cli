# 开放平台接口管理

用户要求接入或配置接口时使用。管理会话与员工调用会话分别登录和核权；本人具有管理角色不等于用户授权任意发布或扩大授权对象。复用当前明确的环境、系统、接口和授权范围。

需要管理登录时由Agent运行`platform manage login --profile <管理记录> --environment <prod或stg>`并保持进程，本人完成浏览器验证。未明确环境时默认prod；平台入口由应用固定，管理记录与业务记录分别保存。

## 从定义到计划

先调用 `platform manage capabilities`、`systems`、`drafts`；列表按返回页码继续，不能把第一页当作全部接口。已有系统复用完整配置与当前摘要。已有接口核对草稿修订、已发布版本、来源冲突和已有内部授权，保存原状态。

使用当前环境在线 OpenAPI，或有版本证据的原接口定义。仅保留选中路径与递归依赖模型，移除普通参数列表中的认证头声明；认证由路由负责。不能从示例数据猜字段、成功规则或读写性质。Swagger 存在泛型、日期、可空或循环引用问题时，按原具名模型核对并记录调整；不把未解析模型改成任意字典。

将 UTF-8 清单保存到文件，再调用：

```sh
dt-cli platform manage plan --profile <management-profile> --manifest <manifest.json>
dt-cli platform manage inspect --profile <management-profile> --id <planId>
```

完整清单的类型示例；示例系统、路径和授权对象不用于实际应用，实际值来自本次已确认范围与系统回读：

```json
{
  "protocolVersion": 1,
  "environment": "stg",
  "system": {
    "systemId": "example-system",
    "title": "业务系统示例",
    "environment": "stg",
    "sourceUrl": null,
    "allowedOrigins": [],
    "route": {
      "upstreamBaseUrl": "https://gateway.example.com/example-system",
      "authenticationMode": "NATIVE_AUTH",
      "credentialHeader": "Authorization",
      "credentialPrefix": "",
      "timeoutMillis": 15000
    },
    "template": null,
    "enabled": true
  },
  "source": {"kind": "FILE", "document": "<OpenAPI JSON原文>"},
  "selections": [{
    "sourceKey": "GET /example",
    "classificationConfirmed": true,
    "alias": "<业务别名>",
    "acknowledgeSource": false
  }],
  "grants": [{"version": 0, "principalType": "USER", "principalId": "<明确对象>", "enabled": true}],
  "carryInternal": false
}
```

已有系统的 system 对象完整复用当前配置，不能将其模板、认证模式或路由替换为示例值。结构字段由当前管理协议决定，实际 source.document 放经核对的 OpenAPI 原文；`CONFIGURED` 来源使用当前已登记地址且 document=null。新系统的源站、认证方式、头、超时、允许范围和模板逐项核对。

计划 `NOT_READY` 时根据 `issues` 补齐明确读写分类、成功规则、Schema、视角、数据范围、分页和限额。单个 selection 的 `overrides={adjusted:<完整定义>,fields:[<字段枚举>]}` 只覆盖本次确认字段。

接口业务说明同时记录：精确/包含/语义匹配方式，状态与日期组合，空值/省略含义，字段是否成对生效，枚举的业务意义，上游结果ID到下游输入的关系，默认排序及不能支持的排序。类型、必填、枚举、范围和已明确的联动尽量由契约校验；用最窄正常/反例核对实际筛选，不把“请求成功”当作条件生效。修正示例文字时同时核对数值约束，避免“最多5条”文字与已发布100条上限矛盾。

分页枚举为 `PAGINATION_KIND`、`PAGINATION_PLACEMENT`、`PAGINATION_PAGE_PARAMETER`、`PAGINATION_SIZE_PARAMETER`、`PAGINATION_ITEMS_POINTER`、`PAGINATION_TOTAL_POINTER`、`PAGINATION_NEXT_POINTER`、`PAGINATION_FIRST_PAGE`、`PAGINATION_PAGE_SIZE`；没有 `PAGINATION` 聚合枚举。正文分页参数使用 `/pageNum` 等 JSON Pointer。

## 应用与验收

核对 READY 计划的本人、环境、所选路径、原修订与摘要、目标版本、路由及授权对象。来源冲突只在实际比较后设置 `acknowledgeSource=true`；它不是通用消错开关。只发布当前任务对应变更，保留其他草稿。

应用前保存 planId、digest 和一个随机 requestId：

```sh
dt-cli platform manage apply --profile <management-profile> --id <planId> --digest <digest> --request-id <原requestId>
dt-cli platform manage status --profile <management-profile> --id <planId>
```

超时或丢响应先回读原计划。只有服务端允许恢复原请求时才用原 requestId 和 `--retry`，不能换键盲重试。

APPLIED、授权投影和员工真实调用分别核对。使用员工会话 `resolve` / `describe` 固定版本，验证正常读取与本次相关边界。配置写接口不代表授权发送测试业务数据；用户未给出真实写入指令时验证模板、契约和拒绝边界即可。

较大的来源或计划在代理链路被拒绝时，先缩减到单接口及其依赖模型生成计划。只在输入确有变化后继续，不重复提交相同失败内容。记录具体阻塞与恢复条件，不通过令牌或直连业务接口绕过。
