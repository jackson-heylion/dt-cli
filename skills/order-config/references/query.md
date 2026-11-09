# 订货配置接口

系统 `supply-chain-server`，STG 要求见[接口清单](../scripts/requirements.json)，其他环境核对等效约束。

| 别名 | 原接口与用途 |
| --- | --- |
| shops-search | GET `/shops/withAuth`，用 code／name 定向查有权门店，返回 code → shopCode |
| order-config-read | GET `/deliveryCenters/current/shopItems`，直接按门店和品项查配置；列表 /list，总数 /total |

## 选择路径

只有编码时可直接查配置，按编码说明结果。名称与编码首次并列且无可靠对应证据时，按完整 code 查店一次；已有同本人、环境、身份域的真实映射则复用。仅有名称先解析为编码，有冲突才确认。

固定有权配送中心视角；业务拒绝不换中心重试。两接口支持同一视角时按底座规则复用。配置接口原生支持 itemName，无需增加采购价品项搜索；明确品项无结果时保留条件。

## 参数与结果

| 字段 | 含义 |
| --- | --- |
| shopCode | 本 Skill 必传门店编码，字符串 |
| shopName | 原接口可选，不能与 shopCode 同传；已有编码则省略 |
| itemCode | 整数品项编码，须能无损表达完整输入 |
| itemName | 名称筛选；未确认匹配方式时只称候选，返回后核对编码和规格 |
| itemEnable | 品项启用状态，不等于配置可订货 |
| orderable | 配置可订货状态 |
| orderMethod | 必填 CREATE_TIME_ASC／CREATE_TIME_DESC，仅表示创建时间排序 |
| pageNum/pageSize | 按契约及任务总预算，预览默认5条 |

GET 参数交给底座构造器的 query-file，不能将平铺对象直接作为 CLI input-file。品项编码不能无损转整数时，可有据按名称查询后核对完整返回编码；仍不支持则报告缺项，不删前导零或借用采购价 itemId。

| 业务值 | 响应字段 |
| --- | --- |
| 起订量／最大订货量 | minOrderNum／maxOrderNum |
| 订货倍数 | orderNumMultiplier |
| 库存下限／上限 | minSafeStock／maxSafeStock |
| 可订货状态 | orderable |
| 订货单位 | orderUnit |

保留完整 code、name、specification、standardUnit 供辨识。仅加已核实单位，库存单位不套用订货单位；倍数不推断为每次增订数量。未核实的 inventoryType 等字段不展示或猜含义。

## 时间与比较

返回是读取时的当前配置，无历史快照不能回答某日配置。CREATE_TIME_DESC 是最新创建；最近修改需真实 updateTime 与完整目标范围，不能用首页替代。

比较限用户选定门店和同一品项范围。按完整编码对齐，规格／单位不同分别呈现；同店同编码多条冲突保留，不擅取最新一条为执行值。缺值、零、无记录和重复分别处理。

响应未回传门店时保留请求与视角来源；来源无法支撑归属则停止该结论。交付所问配置表及读取时间，标明当前配置；部分读取只比较已覆盖范围。
