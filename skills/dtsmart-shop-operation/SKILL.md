---
name: dtsmart-shop-operation
description: 查询餐饮军师（dt-smart）的本人门店、门店经营、订单、菜品、评价、排班、沽清、客流、排队及渠道和同行对比。用于按门店和日期取数或导出结果。
license: MIT
metadata:
  version: "1.0.0"
---

# 餐饮军师门店经营

使用 `dt-cli` 和所选环境中已确认的本人平台记录。沿用底座 Skill 的登录与失败恢复流程。自动化可读取 [接口依赖清单](scripts/requirements.json)。正式环境与 STG 各发布 30 项只读接口，员工调用须具备相应环境的接口授权。

1. 选接口。用 `python3 scripts/dt_smart.py endpoints` 查命令和平台别名。需要口径、必填条件或返回字段时，再读下表对应的业务参考。
2. 固定范围。已知门店组织编码时直接传 `--shop-codes`。需要解析时，用 `shop-tree` 按门店全名或组织编码核对门店 ID。正式环境的门店树使用账号的首个可用品牌，STG 可指定品牌编码。有同名或重复编码时先确认对象。经营接口始终提交明确门店范围，包括原本可省略门店的沽清查询。
3. 读取。用 [脚本](scripts/dt_smart.py) 传 `--cli`、`--profile`、`--environment` 及所选接口的业务参数。脚本通过 `resolve` 固定本人可见的只读执行版本，再用 `call` 提交 JSON 正文；接口未发布或未授权时按平台错误处理。 `--dry-run` 仅展示拟提交的正文和查店条件。
4. 交付。核对实际日期、门店、渠道、统计粒度和单位；标明空结果、未提供字段和部分分页。需要全量时检查 `complete`，需要导出时用 `--out`。汇总人数、比率、同环比时按相应口径计算。

| 要查的内容 | 参考 |
| --- | --- |
| 经营汇总、订单、赠退菜、菜品、翻台、优惠、同城对比 | [经营数据](references/operations.md) |
| 评价汇总、标签、维度、原文和问卷 | [评价数据](references/evaluation.md) |
| 线上流量、天气、日历、排班、沽清、客流、排队和查店 | [门店基础数据](references/store-data.md) |
| 同行或集团内部品牌经营对标 | [同行对标](references/peer.md) |
| 渠道同环比、首尾单偏差、千单差评率及点评曝光 | [渠道指标](references/channel.md) |
| 启动命令、通用参数和分页 | [调用说明](references/query.md) |
