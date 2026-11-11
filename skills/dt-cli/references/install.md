# 安装与更新

复用已确认且满足 `scripts/distribution.json.minimumCliVersion` 的 launcher。位置未知时，一次检查 PATH 与受管位置，选可执行候选离线 version 一次；未进 PATH 不代表未安装。

| 平台 | 受管程序 |
| --- | --- |
| macOS | `$HOME/Library/Application Support/com.datousoft.dt-cli/installation/bin/dt-cli` |
| Windows | `$env:LOCALAPPDATA\datousoft\dt-cli\data\installation\bin\dt-cli.exe` |

## 自动准备 CLI

仅在缺程序、不兼容或用户要求安装／更新时运行：

```sh
bash <Skill目录>/scripts/bootstrap.sh
```

```powershell
powershell -NoProfile -File <Skill目录>/scripts/bootstrap.ps1
```

退出码0且 ok=true 后使用 `data.launcher` 绝对路径。`data.action` 区分 existing/install/upgrade；更新检查失败但旧版兼容时可继续。明确刷新用 --refresh 或 -Refresh。

脚本和原生安装器负责固定 HTTPS 来源、摘要、长度、兼容性与原子切换；固定源见 distribution.json。校验失败保留现有安装，不换域名、跳过校验或删除锁。安装无需 Python／Node.js。

用户明确维护时可用 `upgrade --online --check` 检查、`upgrade --online` 更新；`upgrade --check` 仅查本地。离线用 `upgrade --package <zip> --sha256 <摘要>`；回退先 `upgrade --rollback --check`，成功再 `upgrade --rollback`。配置与凭证由 CLI 保留，来源有歧义或终端策略拒绝时交组织支持。

WorkBuddy 存储权限失败按[本地存储恢复](recovery.md#本地存储故障)处理，不作为普通查询前置检查。

## Skill 导入

CLI 与 Skill 独立更新；`upgrade --online` 不更新 Skill。先导入兼容 dt-cli 底座，再导入所需业务包；依赖声明不自动补装或赋权。新行为测试前核对客户端实际加载的底座及业务Skill版本，程序版本不能代替Skill版本。

本地 ZIP 用客户端导入，保留同名顶层目录和相对资源。公网已发布版本才用以下命令，目标为绝对路径、父目录已存在：

```sh
<launcher> skill install --directory <目标目录>/dt-cli
<launcher> skill install --name purchase-price --directory <目标目录>/purchase-price
<launcher> skill install --name purchase-price --directory <目标目录>/purchase-price --check
```

缺 CLI 可用随包 `scripts/install-skill.sh --directory <目标目录>/dt-cli` 或 `install-skill.ps1 -Directory <目标目录>/dt-cli` 准备。安装器只更新指定目录，校验来源、版本与摘要，保留恢复备份并拒绝覆盖本地修改。

| 客户端 | 导入入口／目录 |
| --- | --- |
| Codex | `.agents/skills/<name>/`，重新加载后调用 `$<name>` |
| Claude Code | `.claude/skills/<name>/`，调用 `/<name>` |
| WorkBuddy／CodeBuddy | 技能导入入口；支持项目技能时用 `.codebuddy/skills/<name>/` |
| 千问办公 | 扩展 → 技能 → 安装技能，保留 .skill-metadata.yaml |
| 其他客户端 | 技能导入入口，须支持相对引用与本地命令 |

更新后分别核对文件版本和客户端实际加载。用户要求清理时，确认新版可用后再处理当前目标客户端本次更新名称的旧副本与安装恢复备份。保留其他技能、CLI 版本和账号。缺权限时说明需本人完成的具体操作。

手工下载时仅检查已知目标目录，使用新临时目录、先查 ZIP 条目再解压；来源和摘要仍须校验。Windows 使用 PowerShell/.NET 与 -LiteralPath，不混用 Git Bash 和 Windows 路径。安装成功、客户端加载和业务执行分别报告。

## WorkBuddy 实机复验

可直接发给 WorkBuddy：

> 请把 dt-cli 和底座技能更新到最新稳定版，确认新版已加载，用我的账号做一次已授权的只读查询，简短告诉我版本、账号和查询结果。

1. 用同一受管启动器运行 `version`，另核对 WorkBuddy 当前会话实际加载的底座版本。已加载新版就继续。只核对到磁盘文件时，请本人完全退出并重新打开 WorkBuddy，在新会话继续；保留当前进度供接续。
2. 沿用已明确的本人账号记录与环境，使用 `check --profile <记录名>` 核对在线会话。缺少明确绑定时按本人身份和环境定位，必要时用 `profiles`，歧义仍在时请本人确认。`current=true` 仅描述该次检查的记录。缺少匹配记录或会话失效时，按[浏览器登录](login.md)沿原环境和业务用途处理。
3. 用已确认的只读接口、输入和视角做一次实际查询，能力未知时才按[平台调用](platform.md)查找。核对退出码、业务成功与实际范围。用一两句话报告运行版本、本人会话、只读结果和仍待完成的动作。管理用途、写入及清理有各自的验收流程。

启动器路径、子命令和选项分别传入。zsh 用 `"$launcher" doctor --storage`，PowerShell 用 `& $launcher doctor --storage`。若提示命令不存在，先核对实际参数，尤其是带空格的路径及是否把命令和选项拼成一个参数。

存储与目录许可只在相关故障或明确验收时检查，见[本地存储恢复](recovery.md#本地存储故障)。常规网络无法连接时说明查询是否完成并联系企业网络支持。通过现场已有的临时网络路径查询成功时，注明“查询已完成，常规网络仍待复验”。
