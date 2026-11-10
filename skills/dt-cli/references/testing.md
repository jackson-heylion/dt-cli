# 耗时日志与测试报告

仅在用户要求耗时日志或测试报告时使用；业务范围与写入授权不变。复用已有 Python3.9以上及随包工具，普通查询无需此流程。

## 开始与执行

读取入口后、进一步准备前启动：

```sh
python3 <底座目录>/scripts/test_run.py start --directory <新的任务绝对目录> --title <测试名称> --environment stg --conditions <固定条件>
python3 <底座目录>/scripts/test_run.py stage --directory <任务目录> --name 账号与接口准备
python3 <底座目录>/scripts/test_run.py run --directory <任务目录> --name 平台账号 --kind profile --cli --output profiles.json -- <launcher> profiles
```

start 以实际取时为起点，此前部分记未记录；缺 Python 时说明计时缺项，继续可完成的业务。stage 切分墙钟阶段，run 的 `--` 后按参数数组传入，含空格路径是一个参数。输出使用任务目录内新文件，失败也记录且不自动重试。

kind 可选：local（本地操作）、profile（账号元数据）、resolve（契约）、view（视角）、read（读取）、write（已授权写入步骤）。CLI 命令加 --cli，自动保存原输出、最终信封和正文；日志不含完整参数或凭证。

输入按[输入封装](input.md)构造，成功才继续。已有明确修正依据且允许重试时，用新输出并传 `--attempt 2 --retry-reason <原因>`；未知发送只查原记录。

## 独立调用并行

输入均准备完毕后，可用 batch 执行独立解析或读取；依赖步骤与写入保持顺序。计划示意：

```json
{"parallelism":3,"calls":[{"name":"读取名称","kind":"read","cli":true,"output":"result.json","argv":["真实launcher","platform","call","--profile","原账号","--operation","已解析编号","--version","整数版本","--input-file","输入绝对路径"]}]}
```

```sh
python3 <底座目录>/scripts/test_run.py batch --directory <任务目录> --group 主数据 --plan <计划.json>
```

单组最多20项，并发1至4，仍受任务与接口限额约束。报告按实际时间重叠判断并发，相同组名不等于并发。

## 完成

保存简短 business-result.md，将[摘要模板](../templates/test-summary.json)复制后填写实际版本、覆盖、限制和问题；时间、次数及调用表由工具生成。

```sh
python3 <底座目录>/scripts/test_run.py finish --directory <任务目录> --summary <摘要.json> --business <业务结果.md>
```

生成 timing.jsonl 和 test-report.md。总墙钟从 start 到首次报告保存完成；命令毫秒使用单调时钟，不相加替代总耗时。未包装间隙包含模型、调度和本地操作，不能全归于网络。

先给业务结果。用户要可复制报告时，在最后的 Markdown 代码块原样复制生成报告，并给报告及日志绝对路径；不重写计时表，不为计时增加探针或业务调用。
