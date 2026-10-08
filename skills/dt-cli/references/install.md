# 安装、升级与平台适配

运行条件：Agent 能读写任务文件并执行本地命令，IAM 可达；首次授权需要交互终端和系统浏览器，凭证使用 macOS Keychain 或 Windows Credential Manager。远端沙箱无法连接本机服务或安全存储时，导入 skill 不会补足这些条件。

## CLI 安装与维护

从组织向员工提供的可信发行入口获取固定版本内部原生 ZIP 和整个 ZIP 的 SHA-256。选择 macOS arm64 或 Windows x86_64。核对来源与摘要后解压，从原解压目录执行：

```sh
sh ./install.sh --package <archive.zip> --sha256 <archive-sha256>
```

```powershell
.\install.ps1 --package <archive.zip> --sha256 <archive-sha256>
```

本人环境应允许执行组织程序和安装脚本；遇到终端策略拒绝时使用组织支持的分发方式，不关闭保护。安装返回 `pathDirectory`，将此目录加入 Agent 执行环境的 PATH，重新加载终端/客户端后执行 `dt-cli version`。员工不需要 Rust、Java、Node、Python 或 Windows SDK。没有可信发行入口或摘要时，向用户集中索取缺失的分发信息，保留业务任务；不要猜域名、安装不明程序或静默改用候选包。

用户要求升级时获取其指定或组织公布的新版本包，执行：

```sh
dt-cli upgrade --package <archive.zip> --sha256 <archive-sha256> --check
dt-cli upgrade --package <archive.zip> --sha256 <archive-sha256>
dt-cli version
```

只有检查成功才升级；升级后回读当前版本并继续原业务任务。CLI 自身的 profile、凭证和 skill 不因升级改变。仅执行 `dt-cli upgrade --check` 时得到本地安装状态，`updateAvailable=null` 不代表已检查线上最新版本。用户要求回退或刚升级后程序不可用时，执行 `dt-cli upgrade --rollback --check`，检查成功后按当前用户指令回退；保留版本记录使受管入口始终不变。

现有手工安装可从新包执行一次 `install`，把返回的受管 `pathDirectory` 放在旧入口前面，再回读版本。程序继续复用已有 profile 与凭证；不覆盖旧手工文件。内部稳定版本（release）、候选版本（candidate）有独立安装目录；仅在用户明确要求受控候选试用并提供可信 ZIP 摘要时使用 `--candidate --sha256 <archive-sha256>`。候选生成不证明目标平台运行或内部发行完成。

内部发行不要求 Developer ID、公证、Authenticode 或 `--signer`；旧参数仅保留解析兼容。安装脚本和 CLI 保留 ZIP/程序摘要、平台、版本及构建来源校验。系统仍可能要求 macOS arm64 基本代码签名或按企业终端策略处理程序；遇到系统拒绝时交组织支持处理，不关闭保护。旧版签名安装器如拒绝新包，使用新包安装到新的独立受管目录并调整 PATH，原 profile 与凭证继续复用。

## Skill 导入

Skill 与 CLI 独立安装、升级和回退。复制或导入整个 `dt-cli` 文件夹，保留 `SKILL.md` 与 `references/` 的相对位置。CLI 不修改 Agent skill 目录；用户要求更新 skill 时，显式导入对应新包，保留客户端自身的覆盖规则。

Codex/Claude 使用标准包；WorkBuddy 适配包补 `agent_created`；千问办公适配包补双语字段、推荐任务元数据。各包使用同一个 skill 名称，业务正文与参考相同；每个客户端选择一种，避免同名重复。

| 平台 | 导入方式 |
| --- | --- |
| Codex | 项目相对目录 `.agents/skills/dt-cli/`，重新加载任务后调用 `$dt-cli`。 |
| Claude Code | 项目相对目录 `.claude/skills/dt-cli/`，调用 `/dt-cli` 或描述任务。 |
| WorkBuddy / CodeBuddy | 技能导入入口加载 WorkBuddy 适配 ZIP；支持项目技能的版本可放入 `.codebuddy/skills/dt-cli/`，在技能面板核对。 |
| 千问办公 | “扩展 → 技能 → 安装技能”导入适配 ZIP，保留顶层文件夹、相对参考及 `.skill-metadata.yaml`，从技能列表调用。 |
| 其他 Agent Skills 客户端 | 通过技能导入入口加载同一文件夹，确认支持相对引用和本地命令执行。 |

命令从 PATH 解析，配置、凭证与受信任环境由 CLI 管理。Skill 使用当前用户选择的 profile/环境，不写入机器绝对路径、个人账号、员工 ID、私有域名或凭证。
