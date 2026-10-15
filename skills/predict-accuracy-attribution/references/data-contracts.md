# 接口与数据要求

## 受控取数

所需接口与精确版本在 `scripts/requirements.json`。本版新任务使用 1.1.0 直连接口，IAM 目录提供名称、用途、输入、输出、最低工具要求及登记的业务请求说明。dt-cli 向 IAM / AUTH 获取本人短时身份，随后在本机直接请求业务接口；临时凭证不出现在报告中。发布并授予权限后，`catalog sync`确认当前本人可用接口。

主数据为 `predict.shops.read`、`predict.items.read`、`predict.item-groups.read`、`predict.classifications.read`；报表为 `predict.indicators.read`、`predict.orders.read`、`predict.revenue.read`、`predict.daily-inventory.read`。单日菜品、鲜活配置、日盘历史及仓库查询按原因分析需要读取。

管理员模板使用原 `/predict/queryIndicatorPage`、`/predict/queryPage`、`/predict/predictAmount`、`/predict/queryDailyTheoryInventoryPage`、`/predict/queryDailyDishConsume` 等只读接口。主指标按单店绑定；门店清单与后台可信允许范围相交。仓库先核对原仓库所属门店代码；AUTH门店ID和供应链内部ID不能直接比较。

`/shops/warehouses`可能建仓，`/warehouseInventoryTemplates`可能写配置，不使用这两条作为只读入口。门店组接口只返回第一个已有仓库，不能证明全仓覆盖。

每个本机读取保留 jobId、原视角和接口版本。只有 succeeded 且 complete=true 并满足输出格式，才保存为可用数据。继续任务先查原任务，从已经保存的页恢复；使用已完成本机结果前仍重新核对权限。记录执行位置和取数起止时间，不保证跨页为同一时点快照。失败时单列未完成，不自行更换身份或生成假数据。旧任务继续使用准备时保存的 Skill 和接口版本，原 IAM 后台任务编号保持原查询路径。

## bundle.json

本地计算输入包含：

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
  "coverage": {"complete": true, "issues": []},
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

groups为`{itemCode,memberCodes,complete,source}`。只有完整同组查询可用于继承或排除鲜品。items保留当前品项code/id/specification/standardUnit。

writebackEvidence为`{recordId,status:"complete",source,verifiedAt}`，仅填写可核实的真实证据。原接口没有证据时数组保持空。inventoryRules为`{shopCode,date,operating,requiredWarehouseIds,historicalRuleVerified,allWarehousesVerified}`；规则缺失保持空。

dishConsume为`{shopCode,itemCode,date,items,complete}`；items保留dishSku/dishSpu/recipeCode/dishSpecification/consumeOrderUnit等源字段。actualRevenue、theoryConsume、statisticDate与对应单店同日来源核对。

## 本地记录

run.json保存技能文件摘要、CLI版本和接口版本。analysis-state.json保存本人、环境、品牌、范围、分段、已完成阶段、原jobId和文件摘要。resources保存原程序、资料、分类表和模板，供恢复使用。记录不保存真实访问凭证。
