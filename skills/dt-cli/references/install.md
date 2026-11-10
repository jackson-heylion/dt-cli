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

更新后分别核对文件版本和客户端实际加载。确认新版可用后，仅清理当前目标客户端本次更新名称的旧副本与安装恢复备份，使同名只留一份；保留其他技能、CLI 版本和账号。缺权限时说明需本人完成的具体操作。

手工下载时仅检查已知目标目录，使用新临时目录、先查 ZIP 条目再解压；来源和摘要仍须校验。Windows 使用 PowerShell/.NET 与 -LiteralPath，不混用 Git Bash 和 Windows 路径。安装成功、客户端加载和业务执行分别报告。
