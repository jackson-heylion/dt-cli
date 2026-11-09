# 输入封装

平台请求默认用 `--input-file` 完整封装，不把平铺业务参数直接传入。

## 离线构造

已有 Python3.9以上时使用包内构造器；缺少 Python 可按下节结构生成，无需为简单查询安装运行时。

```sh
python3 <底座目录>/scripts/build_input.py --contract-file <resolve.json> --query-file <参数.json> --view-id <真实视角编号> --object-id <真实对象编号> --output <新输入.json>
```

参数文件为正确类型的业务对象，例如 `{"name":"查询名称","pageNum":1,"pageSize":5}`。按当前契约选择 `--path-file`、`--query-file` 或 `--body-file`，可组合；路径 id 放 path，JSON 正文放 body。无视角时省略成对视角参数。表单、业务头或复杂查询对象按完整封装手工生成。

传本次完整 contract-file 核对参数位置、未知 URL 字段与必填项；完整 Schema、权限和业务语义仍由 CLI 校验。编码保留原字符串，整数和布尔值保持类型。

构造退出码0且 ok=true 后，核对返回 operationId/operationVersion 并使用其新 inputFile。OUTPUT_EXISTS 不会更新旧文件，须换新输出重建；失败即停止依赖调用。

## 固定结构

以下仅为结构示意：path/query/headers/form 是具名参数数组；需要视角时用真实整数 viewId 和字符串 objectId 替换 null。

```json
{"path":[],"query":[],"headers":[],"form":[],"bodyJson":null,"view":null}
```

参数数组元素形如 `{"name":"pageNum","value":1}`，视角形如 `{"viewId":真实整数,"objectId":"真实对象编号"}`。

POST 普通业务 JSON 序列化一次放入 bodyJson 字符串。例如 `{"pageNum":1}` 对应 `"bodyJson":"{\"pageNum\":1}"`；它不是嵌套对象。用序列化器处理转义。

```sh
<launcher> platform call --profile <平台账号> --operation <真实操作编号> --version <整数版本> --input-file <输入.json>
```

确需单独保存正文时，封装 bodyJson 留空，同时传 `--input-file <封装.json> --body-file <普通正文.json>`；二者不重复提供正文。视角始终在封装内。

## 结果解析

保存完整标准输出和退出码；ok=true 后还须核对 businessSuccess、contractMatched 和范围。业务 JSON 在字符串 data.body，按固定契约取字段。

写入可能输出事件行加最终信封，用随包解析器处理 JSON／JSONL，保留原文件：

```sh
python3 <底座目录>/scripts/cli_result.py --input <完整输出> --output <新信封.json>
```

需要正文时加 `--body-output <新正文.json>`；无正文命令省略。测试工具已自动解析，无需重复运行。
