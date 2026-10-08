# dt-cli

员工本人使用的 Rust 原生 CLI：独立登录、授权续期与退出、本人身份与流程消息查询、受控业务接口、后台任务和明确授权的写入意图。业务访问始终由 IAM 及目标系统鉴权。

本仓库用于公开源码构建和客户端分发。源码从维护仓库的已提交版本导出；`source-export.json` 记录来源提交和每个文件的 SHA-256。内部验收资料、部署记录及维护仓库历史不随源码导出。

## 平台与构建

当前版本为 0.4.0，支持 macOS Apple Silicon 和 Windows x64。开发使用 `rust-toolchain.toml` 指定的 Rust 1.94.1 和 `Cargo.lock`；发行程序不要求员工安装 Rust、Node.js 或 Python。

```sh
cargo build --locked
cargo run --locked -- version
cargo run --locked -- help
cargo run --locked -- doctor
```

正常构建只接受 `catalog/environments.json` 编译的可信环境入口，不能通过运行参数替换为任意业务地址。执行业务命令需对应系统已部署 IAM 能力，并由本人在系统浏览器完成登录与范围同意。凭证保存在 macOS Keychain 或 Windows Credential Manager，不进入本仓库或构建日志。

## 原生安装包

维护者在 Actions 中手动运行 **Native build**。流程使用 GitHub 标准 runner，分别在 `macos-15` 和 `windows-2022` 构建、测试、生成 ZIP，并在临时目录真实执行安装器与原生 launcher。构建 artifact 包含 ZIP、manifest、SHA-256、原生检查记录和三种 Skill 导入包，保留 7 天。

ZIP 来自干净提交，包含实际版本与 catalog 来源，安装和升级核对 SHA-256 及平台；内部使用不要求商业发行者签名。操作系统与企业终端的执行策略仍然适用。

Artifacts 是构建产物，尚不是长期自动更新入口。七牛固定 HTTPS 分发地址和 Skill 自动准备能力接入后，员工才可在首次调用 Skill 时自动下载安装。当前通用 Skill 见 [skills/dt-cli](skills/dt-cli/SKILL.md)。

## 验证和权限

```sh
cargo test --locked --lib release::tests
cargo test --locked --test process_contract
python3 -m unittest discover -s tests/release -v
```

CI 不登录员工账号、不请求真实业务数据。写入必须源于用户明确指令；结果未知时查询原 intentId/runId，不重复派发。系统凭证存储、真实浏览器授权和员工设备体验另行验收。
