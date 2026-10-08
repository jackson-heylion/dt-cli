# 本人流程

个人流程 profile 不加 `--system`。按当前 CLI 的 help/Schema 获取应用和筛选命令，先查询本人可用应用和筛选项，再使用有效的 appId、formType 与日期。

```sh
dt-cli whoami --profile <profile>
dt-cli apps list --profile <profile>
dt-cli workflow applications --profile <profile> --kind todo
dt-cli workflow form-types --profile <profile> --kind todo
dt-cli workflow list --profile <profile> --kind todo
dt-cli workflow list --profile <profile> --kind done
dt-cli workflow list --profile <profile> --kind cc
dt-cli workflow list --profile <profile> --kind todo --all
dt-cli workflow open --id <message-id> --profile <profile>
```

默认仅查询一页。用户要求完整整理时才使用 `--all`，并保留 `matchedTotal`、`itemsFetched`、`itemsReturned`、`complete`、截断原因及查询时间。汇总受页数、条数、字节和总时间限制，不承诺快照一致性。

按状态、应用、时间或用户指定标准整理实际记录。可选字段为 `null` 时说明未提供；到达时间不当作发起时间。只打开用户明确选择的消息，不根据列表自动逐项启动浏览器。`browser_requested` 仅证明已请求浏览器交付。
