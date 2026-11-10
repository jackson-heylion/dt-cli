---
name: predict-accuracy-attribution
description: 分析门店或大区的预估准确度、盘点、营业额预估和耗用差异，生成周报或核对异常原因。
metadata:
  version: "1.4.2"
---

# 预估准确度分析

读取已安装的 `dt-cli`（最低版本见 scripts/distribution.json）。只通过已登记的只读接口取数，使用本技能原程序计算。

1. 固定有权品牌、门店／大区和日期。默认北京时间；“上周”为上一完整周一至周日，“近7天”排除今天；“最近”无周期和既定口径时补问。
2. 按[平台取数](references/platform.md)读取目标期数据。三类主数据均空时，只核对目标期必要盘点证据，按[无数据说明](templates/no-data-report.md)交付；不回溯或查询其他门店找数据。
3. 有数据时按[数据要求](references/data-contracts.md)组装输入，按[计算与证据规则](references/calculation-rules.md)计算。需要 Python至少3.9；缺少环境时保留进度，不手算替代。
4. 按[业务报告](templates/business-report.md)交付盘点、营业额预估、订货量准确度及有证据的原因。`templates/report.md` 仅为内部计算记录。

正式排名须有完整覆盖、周期结束和实际回写证据；不足时仅给观察结果。历史应盘规则或全仓证据不足不能认定缺盘；无原因证据写“暂无法判断原因”。计算、分类与阈值沿用原程序。

保存本任务的程序／规则／资产快照、版本摘要、来源与范围，恢复时复用原 jobId 和已保存页。已有受控输入可用 `scripts/analyze.py --input <bundle.json> --output <报告目录>` 重算。


报告默认本地保存，导出、上传或发送按用户要求执行。
