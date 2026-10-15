---
name: predict-accuracy-attribution
description: 分析门店或大区的预估准确度、盘点情况、营业额预估、鲜品和非鲜品差异，并核对可能原因。用户要求预估准确率周报、预估订货量归因或异常门店分析时使用。
metadata:
  version: "1.1.0"
---

# 预估准确度分析

通过 dt-cli 查询本人有权查看的数据，由本技能程序计算，交付短摘要、完整中文报告和结构化结果。

## 开始任务

确定一个有权品牌、日期和门店或大区。日期默认按北京时间解释；“上周”为上一完整周一至周日，“近7天”排除今天。缺少品牌、日期或范围有歧义时集中补问。

使用现有业务 profile（已登录账号的本地名称）。读取[接口与数据要求](references/data-contracts.md)核对可用数据。新任务执行：

```bash
python3 <本技能目录>/scripts/run.py --profile <profile> --brand <品牌编号> --period 上周 --client Codex
```

支持 `--start YYYY-MM-DD --end YYYY-MM-DD`、多个 `--shop <门店代码>` 或 `--region <大区名称>`。WorkBuddy 使用 `--client WorkBuddy --skill-directory <桌面端实际技能目录>`。

程序在新任务前选取技能最新 stable，工具版本不足时更新 dt-cli，并保存本次程序、资料和接口版本。返回 `needs-client-read` 后，当前 AI 工具读取其 `entryFile`，核对技能版本与规则，再使用该任务保存目录中的程序继续：

```bash
python3 <resourcesDirectory>/scripts/run.py --resume <runDirectory> --profile <原profile> --client Codex --confirm-read
```

`--confirm-read` 记录当前工具确实读取原任务内容。文件安装成功和工具实际读取版本分别核对；这个记录不证明真实员工验收。

## 计算与解释

已确认规则以[计算与证据规则](references/calculation-rules.md)和本任务保存的 `scripts/rules.py` 为准。长日期按每段最多31天读取；比较基准另外查询此前14个有效营业额日。任务达到限额或某页失败，保留已完成部分、未完成范围和原 jobId。

报告保留盘点、营业额预估、订货量准确度三块内容和两层原因分析。分类表为附件84项 v2，结合当前品项与完整同组查询核对。未知、冲突单列。鲜品分类与鲜活库存配置分别使用。

正式排名需要周期结束、实际数据回写完成证据和各指标的有效比较值。证据不足写“回写完成未核实”，交付观察统计。数值差异与可能原因分开；天气、节假日、盘点责任、真实耗损需额外证据。历史应盘规则或全仓覆盖不足时，仅写“未查到已完成日盘记录”。

已有受控数据文件可用 `scripts/analyze.py --input <bundle.json> --output <报告目录>` 重算；输入格式见[接口与数据要求](references/data-contracts.md)。说明真实接口数据、构造数据及覆盖范围，使用原程序产生的数值。

## 完成与继续

交付程序返回的短摘要，以及报告、结构化结果和原任务继续入口。继续任务使用保存的资料目录，再查当前权限；原任务本人、环境、品牌和授权保持一致。

默认在本地保存报告。Excel、PDF、发消息或上传由用户另外提出。供应链接入仅通过已登记的只读接口，不修改供应链代码、响应、SQL或数据。
