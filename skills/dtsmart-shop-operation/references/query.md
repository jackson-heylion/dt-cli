# 调用说明

系统为 `dt-smart`。以员工可见的固定契约为准，接口别名为 `shop-operation-<子命令>`。当前包使用根命令版 `dt-cli`，最低 0.8.0。脚本需要 Python 3.9 或更高版本。执行前从 `dt-cli profiles` 固定本人记录和环境；未指定环境时脚本按正式环境核对契约。

从本 Skill 根目录运行：

```sh
python3 scripts/dt_smart.py endpoints --format table
python3 scripts/dt_smart.py shop-tree --environment stg --profile "本人STG记录" --brand-no 1 --shop-codes 99999
python3 scripts/dt_smart.py date-calendar --environment stg --start 2026-11-01 --end 2026-11-02 --dry-run
python3 scripts/dt_smart.py date-calendar --cli "/path/to/dt-cli" --profile "本人STG记录" --environment stg --start 2026-11-01 --end 2026-11-02
```

| 选项 | 用法 |
| --- | --- |
| `--start`、`--end` | `yyyy-MM-dd`，含首尾两天。按接口参考核对最大跨度 |
| `--shop-codes` | 逗号分隔的组织编码，保留前导零。它和门店 ID 是两种标识 |
| `--shop-name` | 门店全名，逗号分隔。经营查询自动解析时，每个名称都须在本人门店树中唯一命中 |
| `--brand-no` | STG 查店可指定一个本人有权限的品牌编码。正式环境按本人首个可用品牌查店 |
| `--param K=V` | 补充发布契约允许的正文参数，可重复。数组用逗号分隔或 JSON；保留数字形状的字符串时用 JSON 字符串 |
| `--page`、`--size` | 分页从 1 开始，单页最多 200。按实际任务控制页长 |
| `--no-all-pages`、`--max-pages` | 单页预览或限制页数，缺省最多读取 50 页，并受契约限制 |
| `--fields`、`--limit`、`--out` | 指定结果字段、终端预览行数或写出 JSON / CSV；单对象响应写 JSON |

`shop-tree`（别名 `shops`）列出餐饮军师当前组织树中本人可见的门店，可按名称或编码精确筛选。输出保留门店 ID、编码、名称和分组路径。组织树没有省市区字段。门店树许可与经营接口许可独立。

每页都经 `dt-cli` 核权，并验证 `businessSuccess`、`contractMatched` 及业务状态。业务正文是 `status=SUCCESS`、`code=200`、`data`。空值可能省略。分页数组在 `data`，页数以顶层 `totalPage` 为准，`totalNum` 的计数单位依契约，翻台率按门店计。只取部分页、遇到失败或上游总数变化时保留已验证页并返回 `complete=false`。终端预览省略的行仍可在同次导出中读取。

金额按接口标注的元或其他单位处理。比率不直接相加，跨店汇总按分母加权；跨日期曝光人数可能重复，累计优先用曝光次数。未回传与实际为零分别表达。
