# 渠道指标对比

| # | 接口 | 说明 | 分页 | shopCodeList |
|---|---|---|---:|---|
| 27 | `POST /shopOperation/channelMetricCompare` | 渠道单指标同比/环比对比（周期×渠道×餐段，15 指标选 1） | 否（`ResponseDTO` 单对象） | 必填 |
| 28 | `POST /shopOperation/channelBusinessHoursDeviation` | 渠道首单/尾单时间与动态基准偏差（按日） | 否 | 必填 |
| 29 | `POST /shopOperation/channelBadRatePerThousand` | 渠道千单差评率同环比（周期×多渠道，一次返回） | 否 | 必填 |
| 30 | `POST /shopOperation/exposure` | 门店大众点评曝光/访问量同环比（周期×门店，一店一行） | 否 | 必填 |

> 本组四接口**均非分页**，返回 `ResponseDTO`；通用限制：日期跨度 **≤92 天**（含首尾，超限 FAIL 信封）、`shopCodeList` **≤1000** 个门店。
> 环比口径全组一致：**对照期 = 本周期往前推 N 个自然日（N = 本周期自然日天数）**，紧邻本周期、结束于 `startDate` 前一天。
> 🔴 **环比不是「上月同期」**：两期天数恒等但跨月时对照期横跨两个自然月——如 2026-10-01~10-31 的对照期是 **2026-08-31~09-30**（不是 09-01~09-30）；要上月同期口径的调用方请自行另算。
> 本组接口**只返回本期值/环比对照值/同比对照值三个数值，不返回变化率**；需要变化率按 `(本期−对照)/|对照|×100` 自行计算（对照为 0 时无意义）。

**27) `/channelMetricCompare`**
- 请求体 `ChannelMetricCompareReq`（**全必填**）：`startDate`/`endDate`/`shopCodeList`、`channel`(`DINE`堂食/`TAKEOUT`外卖/`SELF_PICK`自提)、`timeSlot`(`AllDay`全天/`Lunch`午市/`Afternoon`下午/`Dinner`晚市/`Night`夜宵/`Other`其他)、`metric`（**一次只查一个**，15 选 1）。
- `metric` 指标清单与可用渠道：

  | 指标 | 含义 | 单位 | 可用渠道 |
    |---|---|---|---|
  | `ACTUAL_AMOUNT` | 实收金额（周期累计） | 元 | 堂食/外卖/自提 |
  | `SHOP_DAILY_AVG_ACTUAL_AMOUNT` | 店日均实收 | 元 | 堂食/外卖/自提 |
  | `ORDER_COUNT` | 订单数（周期累计） | 单 | 堂食/外卖/自提 |
  | `SHOP_DAILY_AVG_DISCOUNT_AMOUNT` / `SHOP_DAILY_AVG_DISCOUNT_NUM` | 店日均优惠金额/份数 | 元 / 份 | 堂食/外卖/自提 |
  | `SHOP_DAILY_AVG_REFUND_AMOUNT` / `SHOP_DAILY_AVG_REFUND_NUM` | 店日均退菜金额/份数 | 元 / 份 | 堂食/外卖/自提 |
  | `CUSTOMER_FLOW` | 用餐人数（周期累计） | 人 | **仅堂食** |
  | `SHOP_DAILY_AVG_GIFT_AMOUNT` / `SHOP_DAILY_AVG_GIFT_NUM` | 店日均赠菜金额/份数 | 元 / 份 | **仅堂食** |
  | `PER_CAPITA_ACTUAL_AMOUNT` | 人均 | 元 | **仅堂食** |
  | `PER_ORDER_ACTUAL_AMOUNT` | 单均（实收金额÷订单数） | 元 | **仅外卖**（外卖无用餐人数口径；堂食/自提请用人均） |
  | `ENTRY_RATE` | 进店率（用餐人数÷门口客流×100） | % | **仅堂食** |
  | `TABLE_TURNOVER_RATE` | 翻台率（订单数÷桌台数） | 次 | **仅堂食** |
  | `DINING_DURATION` | 消费时长（总时长÷订单数÷60） | 分钟 | **仅堂食** |

  查询渠道不支持的指标（如外卖查翻台率）返回 **FAIL 信封**并提示该指标支持的渠道，不查库。
- 同比口径：取渠道 ADS 表 `ly_` 预计算字段（去年多个对照日按营业天求均值的店日均口径，按本周期营业日逐日累加）。
- 店日均分母 = 营业门店日数 `SUM(is_business)` 按「门店×营业日」累加（多门店 = 门店数×天数，如 2 店各 7 天 = 14），**不是自然日天数**。
- 返回 `ChannelMetricCompareVO`（`data` 单对象）：
  - 维度：`channel`/`channelName`/`timeSlot`/`startDate`/`endDate`/`compareStartDate`/`compareEndDate`（环比对照期两端）。
  - 对账用分母：`currentBusinessShopDays`/`monthOnMonthBusinessShopDays`/`yearOnYearBusinessShopDays`（本期/环比/同比三段各自的营业门店日数；两期因闭店/新开可能不等）。
  - 指标：`metricCode`/`metricName`/`unit` + `currentValue`/`monthOnMonthValue`/`yearOnYearValue` 三个数值。
- **陷阱**：
  - **比率类指标（店日均/人均/单均/进店率/翻台率/消费时长）某期分母为 0 或缺失时该期为 `null`（键缺失）**——表示该期比率**不可用**（如去年未装 Hik 客流设备时进店率同比为 null），**不得按 0 处理**（(本期−0)/0 会得 Infinity，且会误读为同比持稳）；应跳过该期变化率计算并标注「不可用」。
  - 累加值指标（实收金额/订单数/用餐人数）无数据返回 **0**（非 null）。
  - 两期营业门店日数不等 → **店日均环比变化率 ≠ 累计值环比变化率**，两者不互为校验；对账用返回的三个营业门店日数。
  - 优惠=全量优惠（结账/票券/外卖/套餐，已剔除赠菜优惠）；退菜/赠菜均排除套餐主菜 `is_combo_main=1`。
  - 一次请求 = 一个门店集合 × 一个渠道 × 一个餐段 × 一个指标；跨渠道/跨指标需多次调用。

**28) `/channelBusinessHoursDeviation`**
- 请求体 `ChannelHoursDeviationReq`：`startDate`/`endDate`/`shopCodeList`/`channel` 必填；`timeSlot` **可选**（缺省 `AllDay`，传其他值则按餐段过滤后再取首尾单）；`baselineWeeks` **可选**（缺省 4，取值 1~12）。
- 粒度：`days` 数组一行 = 一个营业日，**覆盖查询周期每一天**（含首尾）；当日无数据时 `firstOrderTime`/`lastOrderTime` 及偏差为 `null`、`shopCount=0`。
- 首尾单算法：先按「门店+营业日」取 `MIN(首单)`/`MAX(尾单)`（渠道 ADS 表 `first/last_order_start_time`），再对当日有数据的门店取**时间均值**。
- **动态基准逐日滚动**：周期内每一天各自的基准 = 该日前 `baselineWeeks` 周内与它**同星期几**的日期的时间均值（按 `pay_date` 的星期几分组，跨零点单归属原营业日）。**不是**固定营业时间表，也**不是**「开始日前 N 周一个固定窗口套住全周期」（周期跨度超过 N 周时后期日期会拿到数周至数月前的基准）。
- 跨零点：时间戳晚于 `pay_date` 的夜宵单按营业日延续折算（时间轴 +24h），跨零点显示为 **24:00 之后**（如 `24:30` = 次日 00:30），首尾单偏差分钟数与展示时间差自洽。
- 返回 `ChannelHoursDeviationVO`（`data` 单对象）：
  - 顶层：`channel`/`channelName`/`timeSlot`/`startDate`/`endDate`/`baselineStartDate`（周期首日的基准窗口起点 = 开始日往前推 N 周）/`baselineEndDate`（结束日前一天）/`baselineWeeks`（实际生效值回传，便于核对取样范围）。
  - `days` 元素：`date`/`weekday`(1=周一..7=周日)/`weekdayName`/`shopCount`(当日参与计算的门店数)/`firstOrderTime`/`firstOrderBaselineTime`/`firstOrderDeviationMinutes`(首单偏差分钟，实际−基准，**正=开市晚**)/`lastOrderTime`/`lastOrderBaselineTime`/`lastOrderDeviationMinutes`(尾单偏差分钟，**负=收市早**)。时间格式统一 `HH:mm`。

**29) `/channelBadRatePerThousand`**
- 请求体 `BadRatePerThousandReq`：`startDate`/`endDate`/`shopCodeList`（1~1000 个）必填；`channelList` 可选，缺省 `["DINE","TAKEOUT"]`，**仅支持 DINE/TAKEOUT**（`SELF_PICK` 自提无评价数据，传入直接 FAIL 信封）；大小写不敏感、去重，返回渠道顺序与入参一致。
- **无餐段维度**：评价表与订单汇总表均无 `time_slot`，误传 `timeSlot` 会被**静默忽略**；因此本指标**不可**与 `/channelMetricCompare` 的 15 个指标放在同一张「渠道×餐段」表格里横向比较。
- 口径：千单差评率 = 差评数 ÷ 订单数 × 1000，单位 **‰（千分率，不是 %）**，2 位小数；低分=3 分及以下评价（按评价时间归属日期）；订单数剔加单 ADD、外卖再剔 ADJUST_ALL；**分子分母各自独立求和**（有差评无订单的店日仍计入分子）；分母为 0 返回 **0.00**（与 QSC 口径一致，不是 null）。
- 同比：按 `dongshi.dim_date_mapping` 参照日折算（一个本期日对应多个去年同日，先按「本期日×渠道×门店」折日均再累加）；日历缺失时同比字段不返回（`null`）。
- 返回 `BadRatePerThousandVO`（`data` 单对象）：`startDate`/`endDate`/`compareStartDate`/`compareEndDate`/`yoyReferenceStartDate`/`yoyReferenceEndDate`（同比参照日端点，日历缺失时键缺失）/`channels[]`。
- `channels` 元素 `ChannelBadRateVO`（一渠道一行；请求渠道无数据仍返回该行，比率 0.00、分子分母 0）：
  - 指标：`currentRate`/`monthOnMonthRate`/`yearOnYearRate`（三段千分率）。
  - 核对：`currentBadNum`/`currentOrderNum`/`momBadNum`/`momOrderNum`/`yoyBadNum`/`yoyOrderNum`（六个分子分母，与三个比率一一对应；**同比分子分母是折算值（参照日日均），可能有小数，属正常**）。
  - `yoySampleDays`：参与折算的「门店×本期日×渠道」组合数，**小于 7 时同比不可信**（通常去年该店该渠道长期未营业或无评价）。
- 🔴 **本接口订单数分母 ≠ `/channelMetricCompare` 的 `ORDER_COUNT`**（实测 DINE 相差约 9%，前者含未登记 `dim_shop` 的门店），对账基准是 QSC 汇总，**两者禁止互校**。

**30) `/exposure`**
- 请求体 `ShopExposureReq`：`startDate`/`endDate`/`shopCodeList`（1~1000 个）必填。
- 粒度：周期×门店，**一店一行**，按 `shopCode` 升序，非分页（`ResponseDTO<List<ShopExposureVO>>`）。
- 返回 `ShopExposureVO`：`shopCode`/`shopName`/`brandId`/`startDate`/`endDate`/`compareStartDate`/`compareEndDate`/`yoyReferenceStartDate`/`yoyReferenceEndDate` + `currentDays`/`momDays`（本期/环比期**有数据天数**，即日均的分母）/`yoySampleDays`（同比有效样本数，0 = 同比不可用）+ 两个指标对象 `exposure`(曝光人数)/`visit`(访问人数)。
- `exposure`/`visit` 为 `ExposureMetricVO`，各含三段值：`currentTotal`/`currentDailyAvg`/`momTotal`/`momDailyAvg`/`yoyTotal`/`yoyDailyAvg`。
- 口径：
  - **`total` 是人次**：源数据人数**按日去重、跨日不去重**，周期累计 = 每日值之和，不是周期内去重人数。
  - `dailyAvg` = total ÷ 该期**有数据天数**（**不是自然日天数**——曝光表存在缺天，如整段缺 2 天则分母少 2；用 `currentDays`/`momDays` 自证）。
  - 同比 `yoyTotal` 是**折算值**（= 同比日均 × 本期有数据天数，用于与本期累计可比），不是去年同期真实累计；同比不可用时该段全为 `null`（键缺失）。
- 🔴 **数据边界**：曝光表只有 **2026-04-24 起**的数据，2026 年内查询同比必然为 `null`（不是 0）；三方门店→内部门店映射覆盖率约 **45%**，**未映射门店不会返回**（查了 10 家店可能只回来几家）。
- 与 `/online-flow`（美团/饿了么外卖流量）是**两套数据**，不可混用。

---
