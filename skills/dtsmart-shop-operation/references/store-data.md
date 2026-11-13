# 本人门店、排班和客流

| # | 接口 | 说明 | 分页 |
|---|---|---|---:|
| 16 | `POST /shopOperation/online-flow` | 外卖线上流量（门店×日期×渠道） | 是 |
| 17 | `POST /shopOperation/weather` | 门店每日天气 | 是 |

**16) `/online-flow`**
- 请求体：`SkillDataBaseReq`（`startDate`/`endDate`/`shopCodeList` 必填）。
- 渠道固定为 `美团` / `饿了么`。返回**完整门店×日期×渠道组合**；无流量数据时各项返回 0。
- 返回 `OnlineFlowVO`：`shopCode`、`shopName`、`date`、`channel`(`美团`/`饿了么`)、`exposureTimes`(曝光**次数**)、`exposureCount`(曝光**人数**)、`enterStoreTimes`(进店**次数**)。
- **`exposureTimes` 与 `exposureCount` 口径不同，不可混用**：前者是**次数**（同一用户多次曝光重复计），后者是**人数**（按日去重）。
- ⚠️ **`exposureCount` 是人数类指标，跨日期/跨渠道不可直接相加**（各日已去重，直接累加会重复计入同一用户）；
  跨日汇总需先按用户去重，本接口不提供去重明细时，请勿用它做周期合计，只作单日单渠道绝对值使用。

**17) `/weather`**
- 请求体：`SkillDataBaseReq`（同上必填）。
- 返回**完整门店×日期组合**；无天气记录时 `weather`/`highTemp`/`lowTemp` 返回 `null`。
- 返回 `ShopWeatherVO`：`shopName`、`shopCode`、`date`、`weather`、`highTemp`(String，原始字段为 text)、`lowTemp`(String)。

### 5.4 经营基础数据（维度/排班/沽清/客流/排队/日历）

| # | 接口 | 说明 | 分页 | shopCodeList |
|---|---|---|---:|---|
| 18 | `POST /shopOperation/dateCalendar` | 日历维度（日/周几/工作日/周末/节假日） | 否 | 无 |
| 19 | `POST /shopOperation/staffSchedule` | 门店排班（门店×日期×岗位×员工类型 上班/理论/差异人数） | 是 | 必填 |
| 20 | `POST /shopOperation/stockoutDishDetails` | 菜品沽清明细-**晚市**（17:00~21:00，门店×汇总对象，默认 POS 分类） | 是 | **可选** |
| 21 | `POST /shopOperation/lunchStockoutDishDetails` | 菜品沽清明细-**午市**（11:00~14:00，结构同晚市） | 是 | **可选** |
| 22 | `POST /shopOperation/passFlow` | 门口客流（门店×统计桶） | 是 | 必填 |
| 23 | `POST /shopOperation/queue` | 门店排队（门店×统计桶，排队数/流失率） | 是 | 必填 |
| 24 | `POST /dim/shopTree` | 本人有权门店的组织架构树 | 否 | 无 |

**18) `/dateCalendar`**
- 请求体：`DateCalendarReq`：`startDate`/`endDate` **必填**（**无** `shopCodeList`、**不分页**）。返回 `ResponseDTO<List<DateCalendarVO>>`。
- 区间内**每天一行**，按日期升序。全局日历维度，不受品牌/门店限制。
- 返回 `DateCalendarVO`：`date`、`dayOfWeek`(1=周一..7=周日)、`isWorkday`(1/0，工作日=非周六日且非法定节假日；**调休补班记为 0**，判工作日以此为准，勿用 `isWeekend=0` 反推)、`isWeekend`(1/0，仅周六日)、`isHoliday`(1/0，仅法定节假日)、`holidayType`(`法定节假日`/`普通日`)、`extraDateFlag`(额外日期标识，取 `dim_date.remark` 日期备注，如寒暑假；有备注则返回备注原文，无备注时为 null，序列化后键缺失)。
- 标记位一律 **1/0 整数**，非布尔。
- **节假日/调休以 `dongshi.dim_date` 维护为准**：做节假日效应、工作日/周末对比、跨年同比前，
  建议先按官方当年放假安排抽样核对（尤其法定节假日连休与调休补班日），确认无误后再批量使用。

**19) `/staffSchedule`**
- 请求体：`StaffScheduleReq`：`startDate`/`endDate`/**`shopCodeList`（必填）**，`page`/`size` 可选。
- 粒度：每行 = 一个「门店 + 日期 + 岗位 + 员工类型」。有排班数据的「门店+日期」固定 **8 行**。
- 顺序：按日期、门店编码升序；同日同店顺序为 前厅(全部/管理组/全职员工/灵工) → 后厨(同上)。
- 返回 `StaffScheduleDailyVO`：`shopCode`、`date`、`postName`(`前厅`/`后厨`)、`staffType`(`全部`/`管理组`/`全职员工`/`灵工`)、`staffCount`(上班人数，**工时折算的等效人数**，2 位小数；空值补 0)、`theoreticalCount`(理论上班人数，2 位小数)、`differenceCount`(差异人数 = 实际 − 理论，正=实际多于理论)。
- **`staffType=全部` 是该岗位合计，包含其余三项，四者不可相加**（跨 `staffType` 求和会重复计数）。
- **理论/差异人数只在 `staffType=全部` 行返回**：理论值取自 `"order".t_attendance_report` + `t_attendance_report_theory_calc`（仅已上报 `REPORTED` 的报告），前厅=`process` 的 `STATION_1.theoryWorkerCount`、后厨=`STATION_2.theoryWorkerCount`——理论值只有岗位级、无员工类型拆分，故管理组/全职员工/灵工行为 `null`；门店当日无已上报报告时理论值为 `null`。
- 门店经 `"order".dim_shop` 按 `(brand_no, shop_id)` **内连接**：排班有记录但门店未登记的行会被**丢弃**（与沽清接口的左连接行为不同）。

**20) `/stockoutDishDetails`（晚市）与 21) `/lunchStockoutDishDetails`（午市）**
- 请求体：`StockoutDishReq`：`startDate`/`endDate` 必填；**`shopCodeList` 可选**（不传/空 = 区间内全部门店）；`summaryType` 可选（不传按 `CATEGORY`）；`page`/`size` 可选。
- **两个接口结构完全同构，仅时间窗口与数据源不同**：
  - 晚市 `/stockoutDishDetails`：统计 **17:00~21:00**，数据源 `dongshi.dws_dish_stockout_daily`；
  - 午市 `/lunchStockoutDishDetails`：统计 **11:00~14:00**，数据源 `dongshi.dws_dish_stockout_lunch_daily`；
  - 两表按沽清时刻**互斥**，同一条沽清记录不会在两份数据中重复统计。
- **汇总方式 `summaryType`**（决定「汇总对象」粒度）：
  | 取值 | 汇总对象 | 说明 |
  |---|---|---|
  | `CATEGORY`（默认） | POS 菜品分类 | 一行 = 门店×分类；`dishNo`/`dishType` 为 null（键缺失） |
  | `SINGLE` | 单菜品 | 一行 = 门店×菜品（`dish_no`） |
  | `COMBO` | 套餐 | 一行 = 门店×套餐（`dish_no`） |
- 默认按平均沽清时间正序（越早沽清越靠前，无时间值排最后）；分页总数按「门店×汇总对象」粒度统计。
- 返回 `StockoutDishVO`：`shopCode`(经 `dim_shop` 由 shop_id+brand_no **左**连接映射，映射不到时为 `null` 且数据仍保留)、`dishNo`(菜品编码，仅 SINGLE/COMBO)、`catName`(CATEGORY 模式=汇总分类名；SINGLE/COMBO=菜品所属分类)、`dishName`(CATEGORY=分类名；SINGLE/COMBO=菜品名)、`dishType`(0-单菜品/1-套餐，仅 SINGLE/COMBO)、`avgStockoutTime`(`HH:mm`)、`avgDailyMinutes`(日均沽清时长，**分钟**，=区间沽清总分钟/该汇总对象有沽清记录的天数，分母非区间自然天数)、`latestStockoutTime`/`earliestStockoutTime`(`HH:mm`)、`stockoutCount`(沽清次数，**不含自动解沽**；可跨行求和)、`latestRestockTime`(最晚上架时间 `HH:mm`，=区间内该汇总对象最晚一次**解沽重上架**时刻)。
- 陷阱：
  - `avgStockoutTime`/`latestStockoutTime` 等为**时间点类**指标，不可跨汇总对象求和；`avgDailyMinutes` 为比率类，同样不可跨行求和。
  - `latestRestockTime` 从未解沽（只沽清未重上架）时为 `null`（键缺失），**不会补 00:00**；它与 `latestStockoutTime` 不是同一条记录的时间点，不构成沽清区间、不可相减。
  - 路由约定：晚市沽清调 `/stockoutDishDetails`，午市沽清调 `/lunchStockoutDishDetails`。

**22) `/passFlow`**
- 请求体：`PassFlowReq`：`startDate`/`endDate`/**`shopCodeList`（必填）**，`granularity`(`DAY` 默认 / `HOUR`)，`page`/`size` 可选。
- 粒度：每行 = 一个「门店 + 统计桶」；按日期、门店编码、小时升序。
- 返回 `PassFlowVO`：`shopCode`、`payDate`、`hour`(`HH`；**仅 `granularity=HOUR` 时返回**，`DAY` 时为 null 且响应体缺失该键；不补空桶)、`passFlow`(门口客流人数 = **入客流 + 经过客流**；空值按 0)。
- 不传 `shopCodeList` 时覆盖区间内全部已登记门店；未在 `dim_shop` 登记的门店 `shopCode` 为 `null`。

**23) `/queue`**
- 请求体：`QueueReq`：`startDate`/`endDate`/**`shopCodeList`（必填）**，`granularity`(`DAY` 默认 / `HOUR`)，`page`/`size` 可选。
- 粒度：每行 = 一个「门店 + 统计桶」；日期按**取号时间**归属（区间含 endDate 当天全天，**非营业日**）。
- 返回 `QueueVO`：`shopCode`、`payDate`、`hour`(取号时间的小时；仅 `HOUR` 时返回)、`queueNum`(排队数，取号条数)、`lossRate`(流失率，%，保留 1 位小数，= 未就餐(`status≠DINED`)取号数 / 总取号数)。
- **`lossRate` 为比率，只在同一门店内可比；跨门店合并需用 `Σ(queueNum×lossRate)/ΣqueueNum` 复算。**

**24) `/dim/shopTree`**
- `shop-tree`（别名 `shops`）通过 dt-cli 发送 `type=GROUP`，读取餐饮军师组织架构树。使用 `--brand-no` 选择品牌，例如太二 `1`；缺省使用上游默认品牌。餐饮军师负责依据本人 AUTH 权限限定品牌和门店。
- Skill 将叶节点展开为 `shopId`（主数据 ID，字符串）、`shopCode`（组织编码）、`shopName`、`groupPath`。经营接口的 `shopCodeList` 使用组织编码，并各自执行权限核验。
- 组织树提供当前门店目录。涉及历史经营数据时，按该接口已验收的统计和权限口径取数。
