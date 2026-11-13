# 经营汇总与订单

| # | 接口 | 说明 | 分页 |
|---|---|---|---:|
| 1 | `POST /shopOperation/businessSituation` | 门店经营情况汇总（门店×渠道×周期，支持 全周期/年/月/日/小时） | 是 |
| 2 | `POST /shopOperation/billDetail` | 门店订单明细（逐单） | 是 |
| 3 | `POST /shopOperation/giftDishDetail` | 赠菜明细（日期×门店×菜品×规格×一/二级原因，堂食） | 是 |
| 4 | `POST /shopOperation/refundDishDetail` | 退菜明细（日期×门店×菜品×规格×一/二级原因，堂食） | 是 |
| 5 | `POST /shopOperation/dishSale` | 菜品销售（数量汇总，按门店分行） | 是 |
| 6 | `POST /shopOperation/dishUpt` | 菜品 UPT（门店×菜品×规格，畅销度） | 是 |
| 7 | `POST /shopOperation/dineTurnover` | 堂食翻台率/翻椅率（按**门店**分页 + 该店按月桌台明细） | 是（单位为门店） |
| 8 | `POST /shopOperation/discount` | 优惠核销（门店×营业日×优惠类型×优惠名称） | 是 |
| 9 | `POST /shopOperation/cityMetric` | 门店指标同城/同模型对比（目标门店×渠道×周期，返回本方值/同城均值/同城最高/同模型均值；`metric` 必填，不支持 HOUR） | 是 |

**1) `/businessSituation` — 经营情况汇总**
- 请求体：`ShopBusinessSituationReq`（`startDate`/`endDate`/`shopCodeList` **必填**，`timeCycle`/`page`/`size` 可选）。
- **跨度限制**：日期跨度 ≤92 天**仅对 `DAY`/`HOUR` 强制校验**；`ALL`/`YEAR`/`MONTH` 为聚合型粒度，**不限跨度**（可跨年查询）。
  超限报 400「查询日期范围不能超过 92 天」。日期格式与「开始不晚于结束」的校验始终生效。
- **时间周期 `timeCycle`**（决定汇总粒度，不传 = `ALL`）：

  | 取值 | 汇总粒度 | `period` 取值 | `payDate` | `hour` |
    |---|---|---|---|---|
  | `ALL`（默认） | 整段 `startDate`~`endDate` 汇总为一条 | `开始日~结束日`，如 `2026-10-01~2026-10-31` | 不返回 | 不返回 |
  | `YEAR` | 门店×渠道×年 | `yyyy`，如 `2026` | 不返回 | 不返回 |
  | `MONTH` | 门店×渠道×月 | `yyyy-MM`，如 `2026-10` | 不返回 | 不返回 |
  | `DAY` | 门店×渠道×营业日 | 同 `payDate`（`yyyy-MM-dd`） | 有值 | 不返回 |
  | `HOUR` | 门店×渠道×营业日×小时（改造前口径） | 同 `payDate` | 有值 | `HH` |

  > 「不返回」= JSON 中**缺省该字段**（等价于 `null`，非空序列化），调用方按缺失处理。

  > ⚠️ **不传 `timeCycle` 时按 `ALL` 全周期汇总**（每个门店×渠道只剩 1 行）；需要逐日/逐小时明细的调用方必须显式传 `timeCycle=DAY` 或 `timeCycle=HOUR`。传非法值（如 `WEEK`）返回参数校验失败。
- 粒度：每行 = 一个「门店 + 渠道 + 统计桶（`period`）」。
- 返回 `ShopBusinessSituationVO`：

  | 字段 | 角色 | 单位/口径 |
    |---|---|---|
  | `shopCode` | 维度 | 门店组织编码 |
  | `billType` | 维度 | 渠道编码：`DINE`(堂食)/`TAKEOUT`(外卖)/`SELF_PICK`(自提) |
  | `billTypeName` | 派生 | 渠道中文名（仅展示；筛选/分组请用 `billType`） |
  | `period` | 维度 | **周期标签，恒有值**，取值见上表（`ALL`/`YEAR`/`MONTH`/`DAY`/`HOUR` 由 `timeCycle` 决定） |
  | `payDate` | 维度 | **营业日** `yyyy-MM-dd`；**仅 `DAY`/`HOUR` 返回，其余周期不返回** |
  | `hour` | 维度 | `"HH"` 24 小时制两位字符串（如 `"08"`/`"23"`）；**仅 `HOUR` 返回，其余周期不返回** |
  | `flowAmount` | 指标 | 流水金额（元，2 位小数；**含加单**） |
  | `actualAmount` | 指标 | 实收金额（元，2 位小数；含加单） |
  | `orderCount` | 指标 | 订单数（单）；**已剔加单**；外卖再剔整单调整订单 `tag=ADJUST_ALL` |
  | `customerFlow` | 指标 | 客流（人）；**非加单订单客流**；外卖再**补回** `ADJUST_ALL` 客流 |
  | `giftAmount` | 指标 | 赠菜金额（元，2 位小数；无赠菜为 0） |
  | `refundAmount` | 指标 | 退菜金额（元，2 位小数；无退菜为 0） |

- **关键口径与陷阱**：
  - **维度请用 `period`**：粗粒度（`ALL`/`YEAR`/`MONTH`）下 `payDate`、`hour` 缺省（不返回），不可作为分组键。
  - `payDate` 为**营业日**，宵夜/凌晨时段（00:00–04:00）归属**前一营业日**。
  - `hour` 统一取订单 `start_time` 的小时：堂食为开台时间；外卖/自提无「开台时间」概念，其 `start_time` 为该单业务时间；**源数据缺失时 `hour` 为 `null`，不得假设恒有值**（`HOUR` 粒度下该行仍会返回，其 `period` 形如 `2026-10-01`）。
  - **行集**：由「订单聚合」FULL OUTER JOIN「赠/退菜金额聚合」得到，只返回**订单侧或赠/退菜侧至少一侧有数据**的组合；仅有赠/退菜的行的 `flowAmount`/`actualAmount`/`orderCount`/`customerFlow` 均为 0，仅 `giftAmount` 或 `refundAmount` > 0；两侧皆无（缺失）的组合视为 0。
  - **`orderCount` 与 `customerFlow` 对 `ADJUST_ALL` 方向相反**：`orderCount` 做「剔减」，`customerFlow` 做「补回」（为对齐 `dws_order_brand_shop_day_channel_add_da`）。**切勿**用 `flowAmount/orderCount` 与 `customerFlow` 交叉校验口径。
  - 客单价正确口径 = `flowAmount / orderCount`（金额含加单、订单数剔加单，加单金额计入但不重复计单）。
  - 排序：`period → shopCode → billType → hour` 升序。

**2) `/billDetail` — 订单明细**
- 请求体：`ShopQueryPageReq`（同上必填，跨度 ≤92 天）。
- 粒度：每行 = 一笔订单（**含加单，不剔除**）。排序：`shopCode` 升序 → `payDate` → 开台时间 → 订单号 **倒序**。
- 返回 `ShopBillDetailVO` 字段：`shopCode`、`billNo`(订单号)、`billType`+`billTypeName`、`memberCardNo`(会员卡号，非会员为空)、`payDate`(营业日)、`startTime`(开台时间 `yyyy-MM-dd HH:mm:ss`)、`payTime`(结账时间)、`flowAmount`/`actualAmount`(元,2位,含加单)、`customerFlow`(人)、`giftAmount`/`refundAmount`(该单赠/退菜金额,元)。

**3) `/giftDishDetail` — 赠菜明细（维度汇总）**
- 请求体：`ShopQueryPageReq`（`startDate`/`endDate`/`shopCodeList` 必填，`page`/`size` 可选，跨度 ≤92 天强制校验）。
- 粒度：每行 = 一个「营业日 + 门店 + 菜品名称 + 菜品规格 + 一级原因 + 二级原因」分组，金额与份数为**组内合计**。
- 口径：**仅堂食（`billType=DINE`）**；赠菜 = 菜品明细中 `gift_num>0` 的行；排除套餐主菜（`is_combo_main=1`）。
- 返回 `ShopGiftDishVO`：

  | 字段 | 角色 | 说明 |
    |---|---|---|
  | `payDate` | 维度 | 营业日 `yyyy-MM-dd`（凌晨 00:00–04:00 归前一营业日） |
  | `shopCode` | 维度 | 门店组织编码 |
  | `dishName` | 维度 | 菜品名称 |
  | `skuName` | 维度 | 菜品规格（无规格时为空） |
  | `giftAmount` | 指标 | 赠菜金额（元，2 位小数，组内合计；可跨行求和） |
  | `giftNum` | 指标 | 份数（组内合计；可跨行求和） |
  | `primaryReason` | 维度 | 一级原因（源表 `gift_desc`，可能为空） |
  | `secondaryReason` | 维度 | 二级原因（源表 `secondary_gift_desc`，可能为空） |

- 排序：`payDate → shopCode → dishName → skuName → 一级原因 → 二级原因` 升序（均为分组键，分页稳定）。
- 注意：维度按**菜品名称+规格+原因**合并，不同 `dish_no` 但名称/规格相同者会并入同一行；求占比时以本接口 `SUM(giftAmount)` 为分母。

**4) `/refundDishDetail` — 退菜明细（维度汇总）**
- 请求体与颗粒度、排序同上（每行 = 一个「营业日 + 门店 + 菜品规格 + 一/二级原因」分组）。
- 口径：**仅堂食（`billType=DINE`）**；退菜 = 菜品明细中 `refund_num>0` 的行；排除套餐主菜（`is_combo_main=1`）。
- 返回 `ShopRefundDishVO`：维度字段同赠菜；指标为 `refundAmount`(退菜金额，元，组内合计) 与 `refundNum`(份数，组内合计)；
  原因取 `refund_desc`(一级) / `secondary_refund_desc`(二级)。

> 对账：两接口均只含堂食，与 `/businessSituation`（全渠道）**不可整表对账**；取 `businessSituation` 中 `billType=DINE` 的行，
> 其 `giftAmount`/`refundAmount` 应与本两接口的 `SUM` 一致（已实测吻合）。

**5) `/dishSale` — 菜品销售**
- 请求体：`ShopQueryPageReq`（同上必填，跨度 ≤92 天）。
- 粒度：每个**有销量的门店**一行（某门店无数据则该店不返回；整体无数据 `data=[]`）。
- 口径：**仅统计非套餐主菜**（排除 `is_combo_main=1`，避免套餐主菜与子菜重复计数）。
- 返回 `ShopDishSaleVO`：

  | 字段 | 单位 | 口径 |
    |---|---|---|
  | `shopCode` | 维度 | 门店组织编码 |
  | `dishNum` | 件 | 菜品销量合计 = `comboDishNum + noComboDishNum` |
  | `comboDishNum` | 件 | 套餐子菜销量（`is_combo_sub=1`） |
  | `noComboDishNum` | 件 | 非套餐菜品销量（`is_combo_sub≠1` 的单品） |

**6) `/dishUpt` — 菜品 UPT**
- 请求体：`ShopQueryPageReq`（同上必填，跨度 ≤92 天）。
- 粒度：每行 = 一个「门店 + 菜品名称 + 规格」。排序：`shopCode` 升序 → `dishNum` 降序 → 菜品名/规格名升序（分页稳定，不重不漏）。
- 口径：仅统计非套餐主菜（排除 `is_combo_main=1`）。
- 返回 `ShopDishUptVO`：`shopCode`、`dishName`、`skuName`、`dishNum`(该菜销量,件)、`orderCount`(该门店订单数,**已剔加单**,同门店内各行同一值)、`upt`(千单件数 = `dishNum×1000/orderCount`，保留 2 位；`orderCount=0` 时为 0)。
- **`upt` 是比率，以门店为单位计算，不可跨门店或跨菜品相加**；门店整体 UPT = `ΣdishNum×1000/orderCount`。

**7) `/dineTurnover` — 堂食翻台率 / 翻椅率**
- 请求体：`ShopQueryPageReq`（`startDate`/`endDate`/`shopCodeList` 必填，跨度 ≤92 天强制校验）。
- **分页单位为「门店」**：一页 = `size` 个门店；`data` 每项 = 一个门店，按 `shopCode` 升序。顶层 `totalNum` = 门店总数、`totalPage` = 总页数。
  请求页越界时 `data=[]`（**不报错**）。
- 返回 `ShopDineTurnoverVO`（单店汇总）：

  | 字段 | 角色 | 单位/口径 |
    |---|---|---|
  | `shopCode` | 维度 | 门店组织编码（源表 `shop_no` 经 `"order".shop` 映射而来） |
  | `shopName` | 维度 | 门店名称 |
  | `dineOrderCount` | 指标 | 堂食订单数（单）；**剔加单**口径 |
  | `dineCustomerFlow` | 指标 | 堂食消费人数（人）；**剔加单**口径 |
  | `tableTurnoverRate` | 指标 | 堂食翻台率 = `dineOrderCount / Σ(月份桌台数×月份堂食营业天数)`，2 位小数 |
  | `seatTurnoverRate` | 指标 | 堂食翻椅率 = `dineCustomerFlow / Σ(月份椅子数×月份堂食营业天数)`，2 位小数 |
  | `businessDays` | 指标 | 营业天数（门店×天，一店一天计 1） |
  | `monthlyShopTables` | 明细 | 该店按自然月的桌台明细数组（按 `yearMonth` 升序），见下表 |

- `monthlyShopTables` 元素 `ShopTableStatVO`：`yearMonth`(`yyyy-MM`)、`shopCode`、`shopName`、`tableCount`(该月桌台数)、`chairCount`(该月椅子数)、`shopArea`(门店面积，平米；源字段为文本，非数值时为 `null`)、`businessDays`(该月营业天数)、`dineOrderCount`/`dineCustomerFlow`(该月剔加单指标，便于按公式复算)。
- **口径与陷阱**：
  - 数据源 `"order".dws_shop_analysis_da`（门店×营业日粒度）；订单数/人数取 `filter_order_count_dine`/`filter_customer_flow_dine`（**剔加单**，不含 `tag=ADD`）。
  - 桌台数/椅子数属**月度属性**，源表按日存在，取该月 `MAX`；营业天数按门店×天累计。
  - **分母逐月加权**：跨月时 = `月1桌台数×月1营业天数 + 月N桌台数×月N营业天数`，**不是**「整段周期的单一桌台数 × 总天数」。因此 92 天内跨月查询结果与「分别按月查再平均」不等价，须按上式复算。
  - 分母为 0（无桌台/无营业天数）时率值为 `0`（不是 `null`）。
  - 只有**堂食**口径，与 `/businessSituation` 中 `billType=DINE` 的 `orderCount`/`customerFlow` 同源，可直接对账（注意 `businessSituation` 需显式传 `timeCycle=DAY` 或 `ALL` 才可比）。
  - 门店面积取自 `"order".dim_shop_table_statistic_v2`（按 `brand_no + shop_code + year_month` 关联）。

**8) `/discount` — 优惠核销情况**
- 请求体：`ShopQueryPageReq`（`startDate`/`endDate`/`shopCodeList` 必填，跨度 ≤92 天强制校验）。
- 粒度：每行 = 一个「门店 + 营业日 + 优惠类型 + 优惠名称」。排序 `shopCode → payDate → discountType → discountName` 升序（分页稳定不重不漏）。
- 返回 `ShopDiscountVO`：

  | 字段 | 角色 | 说明 |
    |---|---|---|
  | `shopCode` | 维度 | 门店组织编码（经 `"order".dim_shop` 由 `(brand_no, shop_id)` 映射） |
  | `payDate` | 维度 | 营业日 `yyyy-MM-dd` |
  | `discountType` | 维度 | 优惠类型（取汇总表 `platform`，如 `优惠券`） |
  | `discountName` | 维度 | 优惠名称（取汇总表 `name`，如 `满100减20`） |
  | `discountAmount` | 指标 | 优惠金额（元，2 位小数，= `SUM(amount)`；可跨行求和） |
  | `discountNum` | 指标 | 优惠数量（个，= `SUM(cnt)`；可跨行求和） |

- **口径（已切换数据源）**：数据来自汇总表 `dongshi.dws_discount_order_his`（唯一优惠口径，含**结账/票券/赠菜/外卖/套餐 5 类**），
  仅按 `pay_date` 收窄，**不区分渠道**——刻意不按 `bill_type` 过滤，堂食/外卖/自提同一优惠项**合并为一行**；
  仅剔除 `platform='赠菜优惠'`（`NULL` 的行保留），避免与 `/giftDishDetail` 重复统计。
- **陷阱**：
  - 已剔除「赠菜优惠」类型，故本接口金额合计 ≠ 订单总优惠额，**不可**用它反推折扣率；赠菜请用 `/giftDishDetail`。
  - `discountNum` 的「个」跨类型语义不完全一致：结账/票券类为**份数**、外卖/套餐类为**订单数**（源表 `coupons_num` 语义差异），跨类型合计时需知晓此差异。
  - 多门店分行不合并，跨门店汇总由调用方按 `shopCode` 分组累加。

**9) `/cityMetric` — 门店指标同城/同模型对比**
- 请求体：`ShopCityMetricReq`（**继承 `/businessSituation` 的 `ShopBusinessSituationReq`**：`startDate`/`endDate`/`shopCodeList` **必填**，`page`/`size` 可选；**额外必传 `metric`**，可选 `timeCycle`）。
- **`metric`（必填，一次只针对一个指标）**：

  | 取值 | 指标 | 口径 |
    |---|---|---|
  | `ACTUAL_AMOUNT` | 实收金额 | 目标区间实收合计 |
  | `SHOP_DAILY_AVG_ACTUAL_AMOUNT` | 店日均实收 | =实收金额/营业天数 |
  | `CUSTOMER_FLOW` | 用餐人数 | 客流人数 |
  | `PER_CAPITA_ACTUAL_AMOUNT` | 人均 | =实收金额/客流人数 |
- **`timeCycle`（可选，不传=ALL）**：`ALL`/`YEAR`/`MONTH`/`DAY`；**不支持 `HOUR`**（传 `HOUR` 或非法值返回参数校验失败）。日期跨度 ≤92 天**仅对 `DAY` 强制校验**，`ALL`/`YEAR`/`MONTH` 不限跨度。`period` 取值同 `/businessSituation`（`ALL`=`开始日~结束日`、`YEAR`=`yyyy`、`MONTH`=`yyyy-MM`、`DAY`=`yyyy-MM-dd`）。
- 粒度：每行 = 一个「目标门店 + 渠道 + 统计周期」。排序 `shopCode → channel → period` 升序。分页单位为结果行，`totalNum`=行总数、`totalPage`=总页数（同公共分页规则：`page` 从 1 开始、`size` 上限 200）。
- 返回 `ShopCityMetricVO`：

  | 字段 | 角色 | 单位/口径 |
    |---|---|---|
  | `shopCode` | 维度 | 目标门店组织编码 |
  | `shopName` | 维度 | 门店名称 |
  | `provinceName` | 维度 | 省 |
  | `cityName` | 维度 | 市 |
  | `shopMode` | 维度 | 门店模型（同模型对比依据，源 `dongshi.dim_shop.shop_mode`） |
  | `channel` | 维度 | 渠道编码：`DINE`(堂食)/`TAKEOUT`(外卖)/`SELF_PICK`(自提) |
  | `period` | 维度 | 统计周期标签（由 `timeCycle` 决定） |
  | `metric` | 维度 | 查询指标编码（回显入参 `metric`） |
  | `metricName` | 派生 | 查询指标中文名 |
  | `value` | 指标 | **目标门店**该指标当前值 |
  | `cityAverage` | 指标 | **同城**门店该指标均值（同品牌+同省+同市） |
  | `cityMaximum` | 指标 | **同城**门店该指标最高值 |
  | `sameModelAverage` | 指标 | **同城且同 `shopMode`** 门店该指标均值 |
- **关键口径与陷阱**：
  - 金额类指标（`ACTUAL_AMOUNT`/`SHOP_DAILY_AVG_ACTUAL_AMOUNT`/`PER_CAPITA_ACTUAL_AMOUNT`）的 `value`/`cityAverage`/`cityMaximum`/`sameModelAverage` ADS 原始单位为**分**，接口统一返回**元、保留 2 位小数**；`CUSTOMER_FLOW` 为人数。
  - 除数为 0（营业天数/客流为 0）时，`SHOP_DAILY_AVG_ACTUAL_AMOUNT`、`PER_CAPITA_ACTUAL_AMOUNT` 对应值返回 **`null`**（非 0）。
  - **`shopMode`/`sameModelAverage` 依赖 `dongshi.dim_shop.shop_mode`**：该字段未维护（为空）时同模型分组匹配不到门店，
    两个字段均为 `null`（**序列化后键缺失**）。此时同城对比只能看 `cityAverage`/`cityMaximum`；
    需要同模型对标请先确认 `shop_mode` 已在门店档案中维护。
  - 比较门店**包含目标门店自身**；结果只返回 `shopCodeList` 指定的目标门店行，后台预筛目标门店所在品牌/城市的候选门店用于算同城、同模型值。
  - 数据源为三张 ADS 日报表（堂食 `ads_order_shop_dine_daily`、外卖 `ads_order_shop_takeout_daily`、自提 `ads_order_shop_self_pick_daily`），先汇总到「门店×日期×渠道」再按 `timeCycle` 聚合，避免 `time_slot` 重复计营业天数；外卖客流取 `ads_order_shop_takeout_daily.customer_flow`。
  - **一次查询只返回一个 `metric`**；需对比多个指标时按 `metric` 多次调用。`cityAverage`/`cityMaximum`/`sameModelAverage` 为聚合统计值，**不可跨行再相加或求平均**。
