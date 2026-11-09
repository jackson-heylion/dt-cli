# 采购价接口与日期映射

系统 `supply-chain-base-report`；STG 最低版本及性质见[接口要求](../scripts/requirements.json)，其他环境核对等效契约。字段语义已按原源码 7740c3052a886e97a84ce947649cba2b2f6bb0c6 核对；运行定义变化时重新核对受影响映射。

| 别名 | 原接口 | 衔接 |
| --- | --- | --- |
| item-search | GET `/srmItem/queryPage` | name／code 模糊检索，取 id、完整code、name、specification |
| purchase-price-query-page | POST `/purchasePrice/queryPage` | 候选 id → itemIds，附范围、状态、日期 |

两者列表为 /list，总数为 /total。名称和编码都不是语义或精确检索；具体商品核对完整编码和规格，类别按[品项范围](item-scope.md)处理。

## 查询范围

默认从真实 views 选 supported=true 的 GROUP，正文传 `customerType=DEFAULT`，查询原接口本人可查的客户范围；不要求选择配送中心，也不猜集团编号。无集团候选时报告权限缺项，不静默改中心。

只有本次明确限定配送中心时，另选已授权且支持 DELIVERY、DELIVERY_CENTER 的契约。不能把 DEFAULT 与旧配送中心视角混用，也不能按仓库名称推断中心归属。

## 参数语义

| 参数 | 含义与联动 |
| --- | --- |
| itemIds | 真实品项 ID 数组，必传非空；单次上限读 Schema，超出才分批 |
| customerType | 全部范围为 DEFAULT＋GROUP；明确中心为 DELIVERY_CENTER＋同一有权 DELIVERY |
| failureState | false 当前未标失效，true 当前已失效，省略查两种；单独 false 不证明有效期内 |
| effectiveRightDate2 | 生效日期≤传入日期，可独立使用 |
| effectiveLeftDate2 | 生效日期≥传入日期，可独立使用 |
| expirationLeftDate | 失效日期≥传入日期，含边界 |
| expirationRightDate | 失效日期≤传入日期，含边界 |
| effectiveLeftDate＋effectiveRightDate | 成对才生效：生效日期≤前者、失效日期≥后者，不是普通起止范围 |
| createLeftTime/createRightTime | 创建时间含边界范围，用于新增 |
| updateLeftTime/updateRightTime | 更新时间含边界范围，用于变更 |

日期格式 yyyy-MM-dd；原 DTO 创建／更新时间为 yyyy-MM-dd HH:mm:ss，不凭 date-time 标签改为 T/Z 格式。价格接口 code 是采购价单号，不能传品项编码；也不支持 itemName。

原接口固定 update_time DESC，无声明的 sort/orderBy 不传。同时间不保证唯一顺序；跨批保留实际排序值，不再按品项或仓库改序。

## 时间口径

D 为固定查询日，默认北京时间当天；[L,R] 含边界。已明确时间字段的近N天默认含今天的N个自然日，时间戳上界为固定查询时刻；明确 N×24 小时则用滚动窗口。“近N天有效”未说明适用、新生效或变更时集中确认。

| 用户口径 | 条件 |
| --- | --- |
| 当前有效／普通采购价 | failureState=false，effectiveRightDate2=D，expirationLeftDate=D |
| 最近有效 | 当前有效条件，按更新时间倒序，不加30天窗口 |
| 有效期与[L,R]相交 | effectiveRightDate2=R，expirationLeftDate=L；状态按用户要求 |
| 覆盖整段[L,R] | effectiveRightDate2=L，expirationLeftDate=R |
| [L,R]新生效且当前仍有效 | failureState=false，effectiveLeftDate2=L，effectiveRightDate2=min(R,D)，expirationLeftDate=D |
| 未来D计划适用价 | 当前未失效且有效期覆盖D，只称计划适用 |
| 历史D实际执行价 | 缺历史失效／执行快照，只能提供当前记录中有效期覆盖D的候选 |
| 明确全部状态 | 省略 failureState，保留用户日期和品项条件 |

## 读取与交付

预览默认 pageSize=5，明确数量优先；预算、分批和全局排序遵循底座查询规划。按生效日期或最低价排名没有服务端排序支持时，需完整覆盖及可比单位，否则只称已核对样本。

核对 itemId 属于候选；明确中心时要求对应客户类型，全部范围允许原接口不同客户类型。当前有效还须逐条满足 failureState=false、effectiveDate≤D≤expirationDate，关键字段为空不能认定有效。返回违背筛选时停止该口径。

适用对象用原响应 warehouseName，门店名称也是原服务在此回填，表头写“适用门店／仓库”。价格保留精度与真实币种；不同单位、规格或对象分别展示。

候选为空停止查价，无价只说明本次条件未查到。完整候选及全部价格批次／分页完成才称“本次检索范围内共X条”，否则称“本次已核对X条”；不足不追加历史或其他状态查询。
