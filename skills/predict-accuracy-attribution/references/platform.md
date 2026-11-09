# 平台取数与字段映射

系统 `supply-chain-server`。按 dt-cli 底座执行，只解析当前分支所需别名；STG 最低版本见[接口要求](../scripts/platform-requirements.json)，其他环境核对等效契约。旧 requirements.json／run.py 仅恢复 governed 任务。

| 数据／别名 | 原接口与输入 |
| --- | --- |
| 门店 shops-search | GET `/shops/withAuth`，code／name、分页 |
| 指标 predict-indicators | POST `/predict/queryIndicatorPage`，shopCodes、orderDateLeft/orderDateRight、分页 |
| 明细 predict-orders | POST `/predict/queryPage`，同上 |
| 营业额 predict-revenue | POST `/predict/predictAmount`，shopCodes、startDate/endDate |
| 理论库存 predict-daily-inventory | POST `/predict/queryDailyTheoryInventoryPage`，shopCodes、statisticLeftDate/statisticRightDate、分页 |
| 品项 predict-items | GET `/items`，code／name、分页 |
| 同组 predict-item-group | GET `/items/{id}/belongingStandardAlternativeItemGroup`，当前品项 id |
| 类目 predict-classifications | GET `/itemCategoryTree` |
| 鲜活配置 predict-fresh-config | GET `/freshConfig/getFreshConfigList`，品项编码／名称、分页 |
| 菜品消耗 predict-dish-consume | GET `/predict/queryDailyDishConsume`，shopCode、itemCode、date |
| 仓库 shop-warehouses | GET `/shops/{id}/warehouses`，门店内部 id 放 path，分页放 query |
| 日盘 predict-inventory-history | POST `/warehouseInventoryBills/history`，warehouseIds、startDate/endDate、type=DAY_INVENTORY、state=FINISHED、分页 |
| 分区 predict-partition-tree | GET `/partitionTree/withAuth` |

## 读取分支

1. 固定有权品牌及目标门店。可靠的同本人／环境／品牌映射可复用，否则按名称或完整编码定向查店；只有大区／品牌任务才枚举对应范围。已选视角按底座规则共用。
2. 每店最多31天一段，将目标期指标、明细和营业额独立并行读取，各自保存完整结果。指标与明细的订货日、到货日分开：当前 STG 必填订货日期；仅按到货日的任务需等效契约，不能猜附加订货窗口。
3. 三类主数据均空时，只核对目标期必要盘点证据后结束；不查询其他门店、年份、品项、同组或基准。单个来源为空只停止该项计算，其余使用已有数据。
4. 有目标记录才补所需理论库存、品项和完整同组，成员全部取得才标 complete=true。分类仍使用84项 v2和同组证据，类目树不能充当鲜品标签。
5. 有可计算万元耗用的目标记录且需要原因比较，才查同店同品项此前14个有效营业额日。按31天窗口复用读取，最多回溯366天、总查询1000次，并服从更窄任务预算；不足14日保留实际日数。
6. 按目标记录补鲜活配置、菜品消耗和目标期盘点。仓库只取该店已有结果，日盘只查目标日期；需后筛时限定该店仓库及预算，不无界翻历史。

成功完整的目标期空结果足以交付无数据说明，失败或部分完成仍为未完成，不另加无日期探针。小样本前5条只证明部分，不能生成完整周报或正式排名；完整取数走底座分页，保留原 jobId、页与停止原因。

## 字段与证据

| 关联 | 核对 |
| --- | --- |
| 门店 code → shopCodes | 必须属于本次有权范围，不省略明确门店 |
| orderDateLeft/orderDateRight | 订货日期，所有分段含边界且一致 |
| arrivalDateLeft/arrivalDateRight | 到货日期，与用户明确订货窗口取交集；缺支持则报告能力缺项 |
| 营业额 predictDay | 业务日期，不等同库存统计日期 |
| 品项 id → 同组路径 id | 成员 code → groups.memberCodes，不借采购价系统 ID |
| 门店内部 id → 仓库路径 id | 核对返回 shopId，不用 AUTH 对象 ID 替代 |
| 仓库 id → warehouseIds | 同一门店、日期与品牌视角 |

当前配置、分区和仓库不能证明历史适用性。历史应盘、经营日和实际回写缺证据时保持对应数组为空，不以时间已过或字段非空替代；null、0、无记录和未取得分别保留。

类目与分区树契约最多六层，超深拒绝响应而非截断。小样本或缺分母不替代原排名与分类算法。

## 本地输入与恢复

仅使用 CLI 已核验的正文及原签名页，按[数据要求](data-contracts.md#本地数据文件)组装 bundle；保留源字段、精度和文件摘要。inventoryHistory 未回传 shopCode 时，只能根据已核对的“门店 → 仓库 → 本次查询”补充来源映射。

保存原计算资源快照，恢复时沿原范围、契约和已读页继续。取数与计算记录留内部，业务报告按入口模板整理。
