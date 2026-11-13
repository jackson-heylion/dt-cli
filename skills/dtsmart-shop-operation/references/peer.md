# 同行品牌对标

| # | 接口 | 说明 | 分页 | shopCodeList |
|---|---|---|---:|---|
| 25 | `POST /shopOperation/peerBrandOpsMetric` | 同行品牌经营指标明细（品牌×期间×口径×指标） | 是 | 无 |
| 26 | `POST /shopOperation/peerBrandOpsNote` | 同行品牌经营数据的备注/口径说明（共 4 条） | 否 | 无 |

> 本组数据为**品牌级**（九毛九/太二/怂=集团内部，小菜园/绿茶=同行），**与门店无关**：不需要 `shopCodeList`，也不涉及 `startDate`/`endDate`；按 `periodLabel`（`24年`/`25年`/`26H1`）等维度过滤。

**25) `/peerBrandOpsMetric`**
- 请求体：`PeerBrandOpsMetricReq`：**全部过滤项可选**（AND 关系）+ `page`/`size` 分页；**无** `shopCodeList`。
  - `brandNameList`：品牌名称列表（精确匹配，多值 IN）；九毛九/太二/怂=集团内部，小菜园/绿茶=同行。
  - `periodLabelList`：期间列表（`24年` / `25年` / `26H1`）。
  - `sectionName`：板块（`营收端` / `成本端` / `经营利润端`）。
  - `compareType`：口径类型（`value`=品牌自身值 / `vs_taier`=源表「太二vs」对照列）。
  - `metricKeyword`：指标关键字（对 `metric_path` / `metric_name` 模糊匹配）。
- 粒度：每行 = 一个「**品牌 × 期间 × compare_type × 指标**」单元格；排序 `sectionName → metricPath → brandName → periodLabel → compareType`。
- 返回 `PeerBrandOpsMetricVO`：

  | 字段 | 角色 | 单位/口径 |
    |---|---|---|
  | `brandName` | 维度 | 品牌名称；九毛九/太二/怂=集团内部，小菜园/绿茶=同行 |
  | `brandScope` | 派生 | `INTERNAL`(集团内部) / `PEER`(同行)；由 `brandName` 派生，源表无此列 |
  | `periodLabel` | 维度 | 期间：`24年` / `25年` / `26H1` |
  | `compareType` | 维度 | `value` / `vs_taier` |
  | `metricPath` | 维度 | 指标全路径（板块/父指标/指标名），唯一标识一行指标 |
  | `sectionName` | 维度 | 板块：营收端 / 成本端 / 经营利润端 |
  | `parentMetric` | 维度 | 父级指标名（顶层指标为 null，键缺失） |
  | `metricName` | 维度 | 指标叶子名称 |
  | `unit` | 维度 | 单位（如 万元/店、元/平/月/店；无单位时为 null，键缺失） |
  | `valueNum` | 指标 | 数值（Double）；源表为文本（`#DIV/0!`、区间值）时为 null |
  | `valueRaw` | 指标 | 单元格原值文本，完整保留原始精度与错误值 |
  | `taierValueNum` | 指标 | 同指标同期间「太二」的值；太二自身行等于 `valueNum`；太二该期间无数值为 null |
  | `isHiddenRow` | 标注 | 源表该行是否处于隐藏状态 |
  | `sourceRow` / `sourceCol` | 标注 | 源表行号/列号（便于回溯源 Excel） |

- **关键口径与陷阱**：
  - 数据源 `dt_smart.ods_peer_brand_ops_metric`；查询**不注入**品牌/门店数据权限。
  - `compareType=vs_taier` 的 `valueNum` **不是本方值**，且**口径不唯一**：实测 530 行中约 **57% 为绝对差**「太二 − 本方」、约 **30% 为差率**「(太二 − 本方) / 本方」（无量纲比值，如 `-1843.9` 表示本方比太二低 1843.9 个单位、`-0.3575` 表示低约 35.8%），其余为本方/太二缺数或公式残值。**勿一律当绝对差**。
  - 精确判定：取同品牌、同期间、同 `metricPath` 的 `value` 行（本方值），若 `valueNum ≈ taierValueNum − 本方值` 则为**绝对差**；若 `valueNum ≈ (taierValueNum − 本方值) / 本方值` 则为**差率**。
  - 源表存在文本错误值/区间（如 `#REF!`、`#DIV/0!`、`3.5%-4.5%`），此时 `valueNum` 为 null，**原文一律以 `valueRaw` 为准**。
  - 「某指标跨品牌对标」的正确做法：在同一 `metricPath`（+ `periodLabel`）下按 `brandName` 横比 `valueNum`。

**26) `/peerBrandOpsNote`**
- 请求体：`PeerBrandOpsNoteReq`：`noteType` 可选（`row_note`=源表 U 列行分组备注 / `source_note`=来源口径说明），**不分页**、**无** `shopCodeList`。返回 `ResponseDTO<List<PeerBrandOpsNoteVO>>`。
- 粒度：每行 = 一条备注，共 4 条（`source_note` 2 条 + `row_note` 2 条）。
- 返回 `PeerBrandOpsNoteVO`：`noteType`、`rowStart`(覆盖起始行)、`rowEnd`(覆盖结束行；单行备注为 null 且键缺失)、`anchor`(源表锚点单元格，如 `U5`/`AJ37`)、`colLetter`(源表所在列)、`noteText`(备注正文)。
- 用途：解释指标口径与取值来源，建议在回答同行对标数据前先取一次作为口径依据。
