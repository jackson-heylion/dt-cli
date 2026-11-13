# 门店评价

| # | 接口 | 说明 | 分页 |
|---|---|---|---:|
| 10 | `POST /shopOperation/evaluation/summary` | 评价按日期×门店汇总 | 是 |
| 11 | `POST /shopOperation/evaluation/detail` | 评价明细（标签以数组返回） | 是 |
| 12 | `POST /shopOperation/questionnaire/detail` | 问卷评价明细 | 是 |
| 13 | `POST /shopOperation/evaluation/tagTree` | 评价标签树（层级 + 正负面方向，不分页） | 否 |
| 14 | `POST /shopOperation/evaluation/tag/statistics` | 评价标签正负面一/二/三级分类计数（按门店，不分页） | 否 |
| 15 | `POST /shopOperation/evaluation/dimension/statistics` | 评价分析维度各取值评论数（门店×取值，不分页，`dimension` 必填） | 否 |

**10) `/evaluation/summary`**
- 请求体：`SkillDataBaseReq`（`startDate`/`endDate`/`shopCodeList` **必填**，`page`/`size` 可选）。
- 粒度：每行 = 一个「门店 + 日期」。**排除** `source` 或 `platform` 为 `QUESTIONNAIRE` 的问卷评价。
- **无评价的日期仍返回**：`starScore`/`tasteScore`/`environmentScore`/`serviceScore` 为 `null`，数量字段为 0。
- 返回 `EvaluationSummaryVO`：`date`、`shopCode`、`shopName`、`starScore`(**平均星级**,1-5,保留 2 位,无评价为 null)、`tasteScore`(平均**口味**评分,保留 2 位,无评价为 null)、`environmentScore`(平均**环境**评分,保留 2 位,无评价为 null)、`serviceScore`(平均**服务**评分,保留 2 位,无评价为 null)、`totalEvaluationCount`(评价总数)、`goodEvaluationCount`(好评数,**`starScore>3`**)、`badEvaluationCount`(差评数,**`starScore<=3`**)、`aiNegativeEvaluationCount`(AI 负面评价数,评价标签含『负面』情感的条评价数)。

**11) `/evaluation/detail`**
- 请求体：`SkillEvaluationDetailReq`（继承 `SkillDataBaseReq`，同上必填）。
- 粒度：**一条评价一条记录**（标签不拆行）；**排除**问卷评价。
- 返回 `EvaluationAnalysisDetailVO`：`date`、`shopCode`、`shopName`、`evaluationContent`(评价原文)、`evaluationTags`(标签数组，无标签为 `[]`)、`evaluationCodeAnalysis`(评价编码分析结果对象；未做编码分析时为 `null`，键缺失)。
- `evaluationTags` 元素 `EvaluationAnalysisTagVO`：`dimension`(一级维度)、`category`(分类)、`categorySub`(二级分类)、`dimensionEvaluation`(情感：`正面`/`负面`/`其他`)、`relatedEvaluationContent`(标签关联的原文片段)。
- `evaluationCodeAnalysis` 元素字段：`motive`(到店动机)、`crowdScene`(客群场景)、`consumptionScene`(消费场合)、`tasteScore`/`environmentAtmosphereScore`/`serviceScore`/`valueForMoneyScore`/`ingredientQualityScore`(体验评分 1~5，评论未提及该维度时为 `null`)、`repurchaseAndPropagation`(复购与传播意愿)、`expectationGap`(期望差)、`iterationPerception`(迭代感知)、`relatedOriginalSound`(支撑结论的原声片段)；文本字段均为中文展示名，未识别时为 `null`。
- 情感取值说明：`dimensionEvaluation` 与 `/evaluation/tag/statistics` 的 `sentiment`、`/evaluation/tagTree` 的 `commentTag` 口径一致，均返回中文（`正面`/`负面`/`其他`），**不是** `POSITIVE`/`NEGATIVE` 英文枚举名。
- 分页总数按**评价数**统计，不按标签行数统计。`evaluationContent`(原文) 与 `relatedEvaluationContent`(片段) 不可混用。

**12) `/questionnaire/detail`**
- 请求体：`SkillQuestionnaireDetailReq`（同上必填）。
- **仅**查询 `source`/`platform` 均为 `QUESTIONNAIRE` 的数据；标签过滤 `comment_tag = 其他`。
- 返回 `QuestionnaireEvaluationDetailVO`：`shopCode`、`shopName`、`date`、`questionnaireEvaluationContent`、`evaluationTags`(同上结构)。

**13) `/evaluation/tagTree` — 评价标签树**
- 请求体：`EvaluationTagTreeReq`：**仅** `channel`（可选，`堂食`/`外卖`；不传返回**全部渠道**标签）；**无** `startDate`/`endDate`/`shopCodeList`、**不分页**。返回 `ResponseDTO<List<EvaluationTagTreeVO>>`。
- 粒度：`data` 每项 = 一个**一级标签节点**，递归结构。
- 返回 `EvaluationTagTreeVO`：`tagName`(标签名，一级分类名)、`commentTag`(**中文取值 `正面`/`负面`/`其他`**，表示该标签情感方向；**仅三级叶子节点返回该字段，其余层级为 `null`**，与 `/evaluation/detail`、`/evaluation/tag/statistics` 的情感口径一致，**不再返回 `POSITIVE`/`NEGATIVE`/`NEUTRAL` 英文枚举名**)、`children`(子节点数组，二/三级标签同结构；**叶子节点 `children` 为 `[]`**)。
- 用途：先取标签树了解系统支持哪些标签分类及正负面方向，再把 `tagName` 传入 `/evaluation/detail` 的 `dineFirstLevelTagList`/`dineSecondLevelTagList`/`dineThirdLevelTagList`（或对应外卖标签参数）做明细筛选；与 `/evaluation/tag/statistics` 配合（树提供可选值，统计提供实际次数）。无匹配标签时 `data=[]`。

**14) `/evaluation/tag/statistics` — 评价标签正负面三级分类统计**
- 请求体：`SkillDataBaseReq`（`startDate`/`endDate`/`shopCodeList` **必填**）；**不分页**，一次性返回全部门店统计。返回 `ResponseDTO<List<EvaluationTagStatisticsVO>>`。
- 粒度：`data` 每项 = 一个门店。
- 返回 `EvaluationTagStatisticsVO`：`shopCode`、`shopName`、`positive`(正面标签统计数组)、`negative`(负面标签统计数组)；元素 `EvaluationTagCountVO`：`sentiment`(评价情感：`正面`/`负面`)、`level`(分类级别：`一级`/`二级`/`三级`)、`value`(标签分类值)、`count`(出现次数，Long)。
- **口径与陷阱**：
  - **排除**问卷评价（`QUESTIONNAIRE`）；标签经评价明细（`source` + 评价ID）关联，**一条评价可命中多个标签，各标签独立计数**。
  - `count` 为指定日期区间、指定门店内该标签值的出现次数，**可跨日期、跨门店求和**；无标签数据时对应数组为空（`positive`/`negative` 为 `[]`）。
  - 因“一条评价多标签”会重复计数，**各 `count` 之和 ≠ 评价条数**；求占比需自行按 `level` 层级确定分母。
  - 与 `/evaluation/tagTree` 配合：树给出可选标签值与本接口层级取值对应，本接口给出各取值实际计数。

**15) `/evaluation/dimension/statistics` — 评价分析维度评论数量统计**
- 请求体：`EvaluationDimensionReq`（继承 `SkillDataBaseReq`：`startDate`/`endDate`/`shopCodeList` **必填**）；**额外必传 `dimension`（非空）**；**不分页**。返回 `ResponseDTO<List<EvaluationDimensionCountVO>>`。
- **`dimension`（必填，一次只统计一个维度）**：取值（**枚举名或中文值均可**，如 `MOTIVE` 或 `到店动机`）：

  | 枚举名 | 中文维度 |
    |---|---|
  | `MOTIVE` | 到店动机 |
  | `CROWD_SCENE` | 客群场景 |
  | `CONSUMPTION_SCENE` | 消费场合 |
  | `EXPERIENCE_TASTE` | 口味 |
  | `EXPERIENCE_ENVIRONMENT` | 环境氛围 |
  | `EXPERIENCE_SERVICE` | 服务 |
  | `EXPERIENCE_VALUE` | 性价比 |
  | `EXPERIENCE_INGREDIENT` | 食材品质 |
  | `REPURCHASE_PROPAGATION` | 复购与传播 |
  | `EXPECTATION_GAP` | 期望差 |
  | `ITERATION_PERCEPTION` | 迭代感知 |

  > **`dimension` 必填非空**：缺失或传空值 → `FAIL` 信封（`code=800`，msg 类似「统计维度(dimension)不能为空…」）；传上表以外的非法值 → 同样 `FAIL`/`code=800`，msg「不支持的评价分析维度: "xxx"。合法取值（枚举名或中文）…」会列出全部合法值。枚举名与中文值等价。
- 粒度：`data` 每项 = 一个「门店 + 维度取值」组合。
- 返回 `EvaluationDimensionCountVO`：`shopCode`、`shopName`、`dimension`(维度中文名，回显入参)、`value`(该维度下的具体取值，如『品类驱动』『聚餐社交』)、`commentCount`(该取值对应评论数量，Long)。
- **口径与陷阱**：
  - 数据源为评价编码分析结果（`dwd_questionnaire_dimension_tag_da`）；**排除**问卷评价。
  - 一条评价可能命中同一维度的**多个取值**，各取值独立计数；**各取值 `commentCount` 之和 ≠ 去重评价数**（不同取值间可能重叠）。
  - 一次只返回一个 `dimension`，多维度需多次调用；`commentCount` 在同一取值内可跨门店求和。无匹配数据时 `data=[]`。
  - **`value` 可能是空字符串或未映射的原始编码**（如 `复购与传播` 下出现 `R8`），评分类维度（口味/环境氛围/服务/性价比/食材品质）
    还可能返回 `value` 为 `null` 的分组（该维度未打分）。聚合前需对空值/未知编码兜底，不要直接当合法取值展示。
