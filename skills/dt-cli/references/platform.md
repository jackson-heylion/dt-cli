# 平台业务任务

## 准备与发现

复用已确认的 launcher、平台 HTTPS origin、环境和本人账号。未知平台地址由组织提供，不猜域名。

从 `platform profiles` 选择匹配平台、环境和稳定本人标识的记录；平台账号不按业务 systemId 拆分。唯一匹配直接使用，多个适用账号有歧义才确认。无记录或实际返回 `AUTH_REQUIRED` 时由 Agent 按[浏览器登录](login.md)发起并保持以下进程，验证成功后继续本次查询：

```sh
<launcher> platform login --profile <平台账号> --origin <https-origin> --environment <环境>
```

新任务只用 platform 账号；个人／governed 记录留给旧任务，不用旧 auth、whoami、doctor 在线探针检查新任务。已有账号无需为新增接口重复登录。

已知别名用 `platform resolve --profile <平台账号> --system <系统> --alias <别名>`；未知能力才定向 search / catalog。只解析当前分支所需接口，完整返回已含契约时无需 describe，缺输入示例才 template。

保存完整 resolve 文件，提取 `data.contract.operationId`、`data.contract.operationVersion`、`data.digest` 及所需参数、分页、限额；编号和版本真实非空才调用。业务清单最低版本只适用于声明环境，其他环境核对等效约束。

需要视角时用 `platform views --profile <平台账号> --operation <id> --version <n>`，从 `data` 数组按业务范围及 `supported` 选择。一次保存 objectName、viewType、viewId、objectId；后续缺编号读原文件。同本人、平台、环境和身份域内，各契约允许该 viewType 时可复用，不能跨域借编号或修改 BOH 当前选择。

业务包的 requiredSkills / minimumDtCliSkillVersion 由 Agent 核对，不代表自动安装或授权。`transport=platform` 清单不交给旧 `skill prepare`；接口整数版本与旧语义版本分开。

## 读取与分页

按[输入封装](input.md)构造文件。保存程序、账号、固定契约、视角和来源，同任务复用；账号或环境变化、版本或视角被拒绝时重新核对相关资料。当前业务数据仍须实际读取，缓存不代表当前权限。

独立解析、搜索和读取可并行，保存各自退出码与完整结果后合并；依赖步骤顺序执行，共享任务预算和限流，不创建子代理。

单次调用的业务正文为 `data.body` 字符串，解析一次后按契约 Pointer 读取；供应链 `/list` 与 HRMP `/data/list` 分开。核对 businessSuccess、contractMatched 及实际范围，HTTP 200 或退出码0不足以证明成功。

完整分页用 `platform jobs start`，传原 profile、operation、version、input-file，保存 jobId。`platform jobs status --id <jobId>` 和 `platform jobs results --id <jobId> --page <n>` 读本机原记录，不加 profile；结果正文在 `data.page.result.body`。保留原签名页，complete=false 时说明已取得部分及停止原因。

工作包使用契约页长；严格只读5条应单次 call 并设 pageSize=5，不能读取20条再只展示5条。后续页不重复登录或发现。

## 明确写入

用户已明确要求写入且对象、参数无歧义时，直接执行一次以下流程；技术 confirm 不要求用户再次点击：

1. `platform writes prepare --profile <平台账号> --operation <id> --version <n> --input-file <输入.json> --instruction <用户原指令>`，保存 recordId 和服务端 intent。
2. `platform writes inspect --id <recordId>`，核对对象、全文、版本和指纹；inspect 不要求含 stage。
3. `platform writes confirm --profile <平台账号> --id <recordId> --fingerprint <原指纹>`，核对获准状态。
4. `platform writes dispatch --profile <平台账号> --id <recordId>`，单次派发并核对业务结果和原记录。

超时、断开或 result_unknown 后只用 `platform writes status --profile <平台账号> --id <recordId>` 核对原 intent，不重建、换键或自动派发；新会话不复活旧确认。

## 恢复原任务

| 情况 | 动作 |
| --- | --- |
| AUTH_REQUIRED | 按[浏览器登录](login.md)在同平台、环境重新登录，稳定本人和身份域保持一致；读取用 `platform jobs resume --profile <原账号> --id <jobId>` 续原检查点 |
| PLATFORM_CONTEXT_REQUIRED / PROFILE_NOT_CONFIGURED | 从实际平台记录选择本人账号；确无记录时按[浏览器登录](login.md)发起，平台地址或环境缺失时集中补齐 |
| 登录 TIMEOUT / open_browser_failed | 按[浏览器登录](login.md#未完成时定位)核对浏览器、页面与回调；保留原任务，原因未改变时停止重试 |
| INTERFACE_PERMISSION_REQUIRED | 将环境、系统、已知接口与当前员工标识交管理员核对“内部授权”；不可见详情不枚举，不创建申请或通知 |
| GATEWAY_REQUEST_DENIED / AUTH_DENIED | 保留拒绝；有明确权限恢复证据才续原读取，写入仍只核对原 intent |
| DEPENDENCY_UNAVAILABLE | 记录实际依赖缺项；仅在明确排障或可区分原因时做一次相关检查，不串联旧入口探针 |
| 账号、视角或原版本不可用／任务达限额 | 保留原进度并报告缺项，不换身份、版本或扩大范围 |

CLI 与 Gateway 执行时继续核权。业务失败、契约不匹配、空数据和部分完成分别说明。
