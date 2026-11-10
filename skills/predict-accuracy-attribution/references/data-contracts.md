# 接口与数据要求

## 受控取数

新任务接口要求在 `scripts/requirements.json`，步骤与源字段映射见[平台取数](platform.md)。dt-cli 通过开放平台与 Gateway 使用本人原业务身份；业务请求不在本机直连原接口。员工目录只证明当前可见的已发布版本，执行时仍核权。

主数据包括本次已核实有权门店、当前分析品项及完整同组；报表包括预估指标、订货明细、营业额及理论库存。当前类目、单日菜品、鲜活配置、日盘历史及仓库按分析需要读取。接口整数版本由平台返回后固定，不使用旧 governed 的 1.1.0 语义版本。

配置复用原 `/predict/queryIndicatorPage`、`/predict/queryPage`、`/predict/predictAmount`、`/predict/queryDailyTheoryInventoryPage`、`/predict/queryDailyDishConsume` 等只读接口。主指标按单店读取；门店清单与本人有权范围相交。门店仓库来源核对原供应链内部 ID 和编码；AUTH 门店对象 ID 与供应链内部 ID 不能直接比较。

`/shops/warehouses`可能建仓，`/warehouseInventoryTemplates`可能写配置，不使用这两条作为只读入口。门店组接口只返回第一个已有仓库，不能证明全仓覆盖。

分页保留 jobId、原视角和接口版本。只有 succeeded 且 complete=true 并满足输出格式，才标记该来源完整；部分页可以保留为部分数据，不能称为全量。继续任务从原已保存页恢复，重核当前权限。记录执行位置和取数起止时间，不保证跨页为同一时点快照。失败时单列未完成，不更换身份或生成假数据。旧任务继续使用准备时保存的 Skill 和接口版本，原 IAM 后台任务编号保持原查询路径。

## 本地数据文件

`bundle.json` 为本地计算输入，以下仅是结构示意；品牌、日期和编码必须替换为本次真实范围。coverage默认按未完成处理，全部必需来源完成后才设true。目标期主数据均空时按入口的无数据分支结束，不为填齐此结构扫描无关主数据或历史。

```json
{
  "schemaVersion": 1,
  "sourceKind": "真实受控接口读取或构造数据验证",
  "context": {
    "subjectId": "本人编号",
    "environment": "环境",
    "brandId": 1,
    "authorizedShopCodes": ["门店代码"],
    "shopCodes": ["门店代码"],
    "startDate": "2026-10-05",
    "endDate": "2026-10-11"
  },
  "coverage": {"complete": false, "issues": []},
  "sources": [],
  "data": {
    "shops": [], "items": [], "groups": [], "classifications": [],
    "indicators": [], "orders": [], "revenue": [], "dailyInventory": [],
    "inventoryHistory": [], "inventoryRules": [], "writebackEvidence": [],
    "freshConfig": [], "dishConsume": []
  }
}
```

源字段名保留：预估指标为predictOrderAccuracy/manualOrderAccuracy/predictConsumeAccuracy/inventoryAccuracy，取值0至1；原值和缺失均保留。原预估id转为recordId，结合shopCode、orderDate、arrivalDate、orderPeriod、itemCode、orderUnit唯一核对指标来源。

shops保留原id/code/name，映射code→shopCode、name→shopName。authorizedShopCodes可为已定向核对的有权子集，不称全量权限；shopCodes必须为其子集。

sources逐项保存系统、operationId、整数版本、contractDigest、视角、查询范围、分页jobId、文件SHA-256和取数时间。coverage.complete仅在全部必需来源及主指标完整后设true。

groups为`{itemCode,memberCodes,complete,source}`。只有完整同组查询可用于继承或排除鲜品。items保留当前品项code/id/specification/standardUnit。

classifications没有明确业务标签时保持空数组，类目树另存来源，不能填默认非鲜品。

writebackEvidence为`{recordId,status:"complete",source,verifiedAt}`，仅填写可核实的真实证据。原接口没有证据时数组保持空。inventoryRules为`{shopCode,date,operating,requiredWarehouseIds,historicalRuleVerified,allWarehousesVerified}`；规则缺失保持空。

dishConsume为`{shopCode,itemCode,date,items,complete}`；items保留dishSku/dishSpu/recipeCode/dishSpecification/consumeOrderUnit等源字段。actualRevenue、theoryConsume、statisticDate与对应单店同日来源核对。

## 本地记录

本次本地交付记录保存本人、环境、品牌、范围、分段、原 jobId、接口版本、文件摘要和计算资源快照。恢复沿用原工作包与已保存页；记录不保存访问凭证。
