# dt-cli

员工本人使用的 Rust 原生客户端：平台登录与会话、接口发现、受控读取、分页工作包、明确指令的单次写入和平台接口管理。业务请求统一经 Gateway，由当前平台许可与业务数据范围共同约束。

命令与参数来自同一根命令树，各级使用 -h / --help。默认标准输出为最终 JSON 信封，进度在标准错误；全局 --format 支持 json/table。

```sh
dt-cli help
dt-cli login --profile work-prod
dt-cli login --profile work-stg --environment stg
dt-cli resolve --profile work-prod --alias <业务别名> --system <系统>
dt-cli call --profile work-prod --operation <接口编号> --version <整数> --input-file request.json
```

新任务未指定环境时默认正式 prod；测试环境明确选择 stg；恢复固定原本人、环境、版本与对象。登录入口由应用固定，凭证仅保存在本机安全目录。管理用途使用 manage login，独立于业务会话。

写入遵循 writes prepare → inspect → confirm → dispatch。结果未知只用 writes status 核对原意图。读取完整页签名落盘后推进检查点，分页不承诺同一时点快照。远端撤销与本地退出分别报告；LOGOUT_PARTIAL 返回退出码7。

支持 macOS Apple Silicon、macOS Intel、Windows x64 原生安装、升级及回退，通用 Skill 用 skill install 安装。源码版本取 Cargo.toml，运行版本取 version；公开 stable 与 Skill 最低要求分别读取固定发行索引和 Skill distribution.json，源码不代表已经公开发布。

```sh
cargo build --locked
cargo test --locked --test platform_root_contract
cargo test --locked --features local-dev --test platform_outcomes --test native_platform_records
python3 -m unittest discover -s tests/release
```

当前 CLI 不提供旧 IAM/governed、本人流程、业务直连或旧任务入口，也不提供 platform 别名。员工设备上的旧专属资料原位保留，当前 CLI 不读取、迁移、恢复或自动删除；平台既有签名记录和完整性密钥继续有效。

使用说明见 [通用 Skill](skills/dt-cli/SKILL.md)，构建与发行参数取当前 scripts/release 和原生 Actions 工作流。公开源码由白名单导出，source-export.json 记录来源与摘要，内部验收和维护历史不随源码导出。构建与本地夹具不替代其他平台原生检查、公开发行回读或真实业务验收。
