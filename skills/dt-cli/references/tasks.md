# 既有工作包恢复

本页只用于已经取得 runId 的旧任务。新业务任务读取对应业务 Skill，使用开放平台接口。

```sh
dt-cli tasks status <原runId>
```

保留原本人、环境、profile、接口版本、参数文件和 jobId / intentId。读取任务按返回的原入口继续；不要把旧任务编号交给 `platform jobs` 或 `platform writes`。

原读取没有 jobId 时如实记录“未定位”，不能重新提交来声称恢复。写入中断、超时或结果未知时只核对原 intent，不重新准备、授权或派发。更换 runId、键或 profile 不能绕过重复内容保护。

恢复记录的本地完整性签名不代表后台执行许可。原本人、授权、合同或视角变化时保留进度，按[恢复](recovery.md)处理。
