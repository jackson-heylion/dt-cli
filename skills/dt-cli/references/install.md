# 安装、升级与平台适配

运行条件：Agent 能读写任务文件并执行本地命令，IAM 可达；首次登录需要本机持续运行的登录进程和系统浏览器，凭证使用当前用户专属本地文件；无需钥匙串确认。远端沙箱无法连接本机服务或安全存储时，导入 skill 不会补足这些条件。

## 自动准备 CLI

缺少已确认可用的 launcher，或用户明确要求安装、检查更新时，运行随包附带的准备脚本。当前会话或交接已提供可用 launcher 时，直接用其绝对路径完成业务调用。

准备命令：

```sh
bash <Skill目录>/scripts/bootstrap.sh
```

```powershell
powershell -NoProfile -File <Skill目录>/scripts/bootstrap.ps1
```

解析退出码与 JSON 信封。成功结果 `data.launcher` 是受管程序的绝对路径，之后的命令用此路径，无需改 PATH 或重启 Agent。`data.action` 表示 existing/install/upgrade；`data.updateCheck` 表示 online/cached/failed-compatible-existing。更新检查失败且旧版满足最低要求时可继续当前业务任务；缺少程序或版本不足则明确失败。明确刷新可传 `--refresh`（macOS）或 `-Refresh`（Windows）。

macOS Apple Silicon、macOS Intel、Windows x64 复用各平台 CLI 默认的当前用户安装目录。准备脚本只用系统 shell、osascript 或 PowerShell/.NET，无需开发运行时。固定 HTTPS 前缀保存在 `scripts/distribution.json`，由维护者和发行物一起发布；Agent 不猜域名、不从索引中的任意 URL 执行代码。未配置下载前缀时报告 `DISTRIBUTION_NOT_CONFIGURED`，保留业务任务。

首次安装先核对 stable、不可变 release 索引和平台 ZIP 的 SHA-256、长度、兼容 schema、干净来源及二进制摘要，再调用原生安装器。后续 `upgrade --online --cached` 每 24 小时检查一次兼容更新，使用 CLI 现有锁、版本目录和原子切换，失败保留已装程序和 previous。四个 Agent 使用同一目录；并发修改冲突按 `INSTALLATION_BUSY` 恢复并重新调用，不删除安装锁文件。

## 显式维护

用户要求线上检查或更新时使用返回的绝对 launcher 路径：

```sh
dt-cli upgrade --online --check
dt-cli upgrade --online
dt-cli version
```

`upgrade --check` 继续只检查本地安装。原有 `upgrade --package <archive.zip> --sha256 <archive-sha256>` 和 `upgrade --rollback` 可离线执行，升级保持 profile、系统凭证及业务恢复记录。自动更新不降级；用户要求回退时用原 launcher 执行 `upgrade --rollback --check`，检查成功后执行 `upgrade --rollback`。

手工安装或旧安装器拒绝新包时，由新包中已经校验的程序建立受管目录。原手工文件保留；如果多个目录或 profile 来源无法唯一绑定，先确认。内部稳定包不要求商业发行者签名；遇到操作系统或企业终端策略拒绝时，交组织支持处理。

## Skill 导入

Skill 与 CLI 独立安装、升级和回退。复制或导入整个 `dt-cli` 文件夹，保留 `SKILL.md`、`references/` 与 `scripts/` 的相对位置。CLI 不修改 Agent skill 目录；用户要求更新 skill 时，显式导入对应新包，保留客户端自身的覆盖规则。

所有客户端使用同一个通用 [dt-cli Skill stable ZIP](https://cdn.jmj1995.com/dt-cli/skills/stable/dt-cli-skill.zip)，此固定地址跟随最新发布的稳定版，后续更新继续使用同一地址。包内同时包含标准 name/description、WorkBuddy 的 `agent_created`、千问的双语显示字段与推荐任务，以及 Codex 的 `agents/openai.yaml`。只保留一个同名 Skill，导入时按客户端覆盖规则替换旧版。Skill 0.5.7 需要 CLI 0.5.5 或更高的兼容版本。

CLI 0.5.4 提供统一安装、更新与回退入口。目录是目标客户端的 `dt-cli` 技能文件夹绝对路径，父目录须已存在；首次缺少 CLI 时先用可信 Skill 包的 bootstrap 准备兼容 launcher。

```sh
<launcher> skill install --directory <目标技能目录>/dt-cli
<launcher> skill install --directory <目标技能目录>/dt-cli --check
<launcher> skill install --directory <目标技能目录>/dt-cli --rollback
```

安装器从固定 HTTPS 源读取 skill-stable、不可变索引和版本 ZIP，验证索引摘要、ZIP SHA-256、字节数、版本及兼容要求，拒绝路径穿越、符号链接与超大文件。同版本且内容一致返回 existing；新版本先完整校验临时目录，再在安装锁内切换并保留 `.dt-cli.skill-previous`，中断时下次调用恢复。已记录目录的本地修改会明确报错，不覆盖。一次返回版本、目录、动作和校验结果；CLI 不扫描或自动更新其他 Agent 目录。

已有兼容 launcher 时直接运行上面的原生命令；否则可用包内 `scripts/install-skill.sh --directory <绝对目录>/dt-cli` 或 `install-skill.ps1 -Directory <绝对目录>/dt-cli` 一次准备并安装。

通过 `channels/skill-stable.json` 获取当前版本及不可变 `releaseKey`，校验该索引的 `releaseSha256`，再用索引中的 `sha256` 和 `bytes` 校验 ZIP。下载期间恰逢发布、固定 ZIP 与索引不一致时，重新读取通道或使用索引中的版本地址；不要跳过校验。需要固定版本时使用 `skills/<version>/dt-cli-skill.zip`。stable ZIP 会随发布更新；已导入的 Skill 需要重新导入，CLI 的 `upgrade --online` 不更新 Skill 文件。

| 平台 | 导入方式 |
| --- | --- |
| Codex | 项目相对目录 `.agents/skills/dt-cli/`，重新加载任务后调用 `$dt-cli`。 |
| Claude Code | 项目相对目录 `.claude/skills/dt-cli/`，调用 `/dt-cli` 或描述任务。 |
| WorkBuddy / CodeBuddy | 技能导入入口加载通用 ZIP；支持项目技能的版本可放入 `.codebuddy/skills/dt-cli/`，在技能面板核对。 |
| 千问办公 | “扩展 → 技能 → 安装技能”导入同一个通用 ZIP，保留顶层文件夹、相对参考及 `.skill-metadata.yaml`，从技能列表调用。 |
| 其他 Agent Skills 客户端 | 通过技能导入入口加载同一文件夹，确认支持相对引用和本地命令执行。 |

命令通过 bootstrap 返回的绝对 launcher 路径执行。配置、凭证与受信任环境由 CLI 管理。Skill 包只包含固定公开下载配置，不包含个人账号、员工 ID 或凭证。

## 下载和客户端加载

下载到新的唯一临时目录。先检查 ZIP 条目，再解压；临时目录无需预先删除。macOS 使用绝对路径。Windows 使用 PowerShell/.NET 和 `-LiteralPath`，下载与解压保持同一种路径形式；Git Bash 的 `/tmp` 不能直接当作 `C:\tmp`。解压与 bootstrap 无需 Python、Node.js 或开发工具。

原生安装器确认文件、版本和摘要。客户端技能列表或加载工具确认 Skill 已加载。当前会话尚未刷新列表时，直接读取已安装的 SKILL.md 和对应参考文件继续任务；按客户端支持的方式刷新。分别报告文件安装、客户端加载和业务查询结果。

隔离安装验收可用 `install-skill.sh --installation-directory <原生安装目录> --directory <技能目录>`，或 `install-skill.ps1 -InstallationDirectory <原生安装目录> -Directory <技能目录>`。日常安装省略原生安装目录，复用当前用户受管安装。
