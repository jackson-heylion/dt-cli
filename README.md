# dt-cli

员工本人使用的 Rust 原生 CLI：独立登录、授权续期与退出、本人身份与流程消息查询、受控业务接口、后台任务和明确授权的写入意图。业务访问始终由 IAM 及目标系统鉴权。

本仓库用于公开源码构建和客户端分发。源码从维护仓库的已提交版本导出；`source-export.json` 记录来源提交和每个文件的 SHA-256。内部验收资料、部署记录及维护仓库历史不随源码导出。

## 平台与构建

当前源码版本为 0.5.3，支持 macOS Apple Silicon、macOS Intel 和 Windows x64。开发使用 `rust-toolchain.toml` 指定的 Rust 1.94.1 和 `Cargo.lock`；发行程序不要求员工安装 Rust、Node.js 或 Python。

```sh
cargo build --locked
cargo run --locked -- version
cargo run --locked -- help
cargo run --locked -- doctor
```

正常构建只接受 `catalog/environments.json` 编译的可信环境入口，不能通过运行参数替换为任意业务地址。执行业务命令需对应系统已部署 IAM 能力，并由本人在系统浏览器登录。后台已有接口权限即可执行，无需逐项同意；新 backend-grant 写合同由后台自动批准并单次派发。凭证保存到当前用户专属文件（macOS/Linux 0700/0600，Windows 当前用户 ACL），不调用钥匙串、不进入源码或构建日志。文件未加密，同一系统账号的进程及管理员可读取；从旧钥匙串版本升级需要登录一次。

## 原生安装包

维护者在 Actions 中手动运行 **Native build**。流程使用 GitHub 标准 runner，分别在 `macos-15`、`macos-26-intel` 和 `windows-2022` 构建、测试、生成 ZIP，并在临时目录真实执行安装器与原生 launcher。构建 artifact 包含 ZIP、manifest、SHA-256、原生检查记录和一个通用 Skill 导入包，保留 7 天。

ZIP 来自干净提交，包含实际版本与 catalog 来源，安装和升级核对 SHA-256 及平台；内部使用不要求商业发行者签名。操作系统与企业终端的执行策略仍然适用。

Skill 首次调用通过随包附带的系统脚本准备原生 CLI，返回绝对 launcher 路径；后续由 `upgrade --online --cached` 检查兼容更新。固定 HTTPS 分发前缀来自 [distribution.json](skills/dt-cli/scripts/distribution.json)，为空时明确报告尚未配置。原有 `upgrade --check` 仍是离线本地检查。

Actions 汇总三个平台的来源、摘要与 Skill 包，生成不可变发行目录。手动勾选 `publish` 时仅从 `main` 的发布作业上传七牛：版本文件不可覆盖，逐个公开回读后最后更新兼容旧客户端的 `channels/stable.json` 和新版的 `channels/native-stable.json`。配置 `QINIU_BUCKET`、`QINIU_REGION`、`QINIU_PUBLIC_BASE_URL` 三个 Actions variables，以及 `QINIU_ACCESS_KEY`、`QINIU_SECRET_KEY` 两个 secrets。URL 必须与源码固定前缀一致；密钥仅进入发布 job。Skill 使用说明见 [skills/dt-cli](skills/dt-cli/SKILL.md)。

## 验证和权限

```sh
cargo test --locked --lib release::tests
cargo test --locked --test process_contract
python3 -m unittest discover -s tests/release -v
```

CI 不登录员工账号、不请求真实业务数据。写入必须源于用户明确指令；结果未知时查询原 intentId/runId，不重复派发。原生文件权限、真实浏览器登录和员工设备体验另行验收。

供应链查询可以为本次请求明确选择本人有权访问的配送中心，也可以给 profile 保存默认中心；每次请求仍由服务端核对授权。CLI 查询不需要修改 BOH 页面上的当前视角。

## 通用 Skill

Codex、Claude、WorkBuddy 和千问办公共用 [Skill stable ZIP](https://cdn.jmj1995.com/dt-cli/skills/stable/dt-cli-skill.zip)。固定地址跟随最新发布的稳定版，安装和更新使用同一地址；当前 Skill 源码版本为 0.5.5，最低 CLI 版本为 0.5.3。Skill 与原生 CLI 独立更新，CLI 使用 `upgrade --online`，Skill 需要重新导入。版本与 SHA-256 从 `channels/skill-stable.json` 指向的不可变索引获取；需要固定版本时使用 `skills/<version>/dt-cli-skill.zip`。授权能力由当前环境 IAM 服务提供。维护者手动运行 Universal Skill release，先核对不可变文件，再更新 stable ZIP、刷新 CDN 并回读，最后更新 Skill stable 索引。

可变对象要求浏览器重新验证缓存，并实际核对公网 Cache-Control 响应头。已发布 Skill 的缓存策略可在 Universal Skill release 中勾选 `repair_stable_cache` 修复；该操作保留包字节、版本和来源，不重新发布同版本 ZIP。

新版原生索引 `releases/<version>/native-release.json` 包含三个平台；原 `release.json` 保留 ARM Mac、Windows 两个平台，版本、提交、摘要和包内容与新版一致。旧 CLI 0.4.1 可从原入口升级，0.4.2 起继续使用三平台入口；缓存保留发布序号防回退，并按入口识别索引摘要。原生公开验证同时检查旧 Skill 0.4.4 和 CLI 0.4.1 的真实升级。

### 钉钉与短信登录

登录页面可选择管理员启用的账号密码、钉钉或短信方式。也可用 `dt-cli auth login --profile me --environment stg --login-method dingtalk`，或添加 `--system hrmp --login-method sms` 登录业务系统。手机验证码在浏览器中输入。管理员在 IAM CLI 工作台的“登录设置”中启用并配置登录方式。
