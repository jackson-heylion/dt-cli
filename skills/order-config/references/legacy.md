# 已有任务

已有 `runId` 的比较继续使用原 profile、版本、参数文件和记录：

```sh
dt-cli tasks status <runId>
dt-cli tasks run order-config.compare --profile <原profile> --version <原版本> --params-file <原参数文件> --run-id <runId>
```

该入口只恢复原任务，不提交新查询。没有 jobId 时保留“未定位”，不能重新提交来冒充恢复。新任务按本 Skill 的开放平台步骤执行。
