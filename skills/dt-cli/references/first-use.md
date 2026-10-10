# First use

## Prepare the launcher

Reuse a confirmed absolute launcher path only when its CLI version meets `scripts/distribution.json.minimumCliVersion`. If it is missing or older, run the bundled bootstrap from [Installation](install.md#自动准备-cli). Installation and the required compatible upgrade are preparation for the user’s task; perform them without asking again. A failed update permits reuse only when the existing CLI meets the minimum.
Use `data.launcher` for all later commands. A new PATH entry or Agent restart is not required.

## Select the account

`profile` is a local name for an account bound to one environment and system.
`selected=false` means no saved selection. It does not mean the account is unusable.
A numeric profile name does not identify the employee or account type.

1. Get the target `systemId` from the task definition or the relevant business reference. Order configuration uses `supply-chain-server`; likes use `hrmp`. Personal workflows use a personal profile.
2. Reuse an account already confirmed for this task. Otherwise read `profiles list --system <system>`; add `--environment <environment>` when known. For personal workflows, select records with `provider=personal` and `systemId=null`.
3. Filter by provider, system and environment. Use a matching saved selection or the single remaining account. Keep an explicit mismatched selection for the user to resolve. If several matching accounts remain, ask which account to use. Show only relevant choices.
4. If no matching account exists, run offline `doctor` to get trusted environments. Use the confirmed environment or the only registered environment. For `stg`, say “查询测试环境”. If several environments remain, ask “查询测试环境还是正式环境？” with the actual choices.
5. Create an unused local profile name for the chosen system and environment. Keep existing account bindings. Start browser login with the absolute launcher path:

```sh
<launcher> auth login --profile <profile> --environment <environment> --system <system> --interaction browser
```

For personal workflows, omit `--system`. `--interaction browser` permits local Agent tools with piped input and output. Password, DingTalk or SMS verification stays in the system browser. The CLI still checks state, issuer, PKCE, account binding and timeout.

The CLI checks local write, atomic replacement and cleanup before opening the browser. On a storage error, follow [storage recovery](recovery.md#本地存储故障), then continue login when the cause is resolved.

Keep the login process alive while the employee uses the browser. Use the tool's running-session support. Success requires exit 0 and `ok=true`; opening a page alone is not success.
Read `auth.login.progress` JSON lines from stderr for checking storage, waiting for browser, exchanging the code, saving and completion. Keep stdout for the final result. Lack of immediate stdout is expected while waiting. Do not diagnose a callback failure from non-TTY input alone.
If an actual tool limitation prevents keeping a local process or opening a local browser, give one complete command with the confirmed launcher and bindings. Continue after the employee completes login.

## Continue the business query

An expired short-lived access token with valid authorization is refreshed automatically by the CLI. Re-login only when its returned auth error requires it.

Successful business login saves its initial catalog. Sync only if the catalog is absent, outdated or the original interface is missing.
`help`, `auth login --help` and offline `doctor` work before login. A business Schema needs the business profile and its catalog.

Resolve a delivery center name with `supply-chain-server.delivery-centers.list` from [Business access](governed.md#供应链配送中心选择与少量读取). Match the user's name against returned objects. Use a unique match; ask only when the choice remains ambiguous.
Keep the user’s shop and item codes. Obtain the center ID, contract version and input fields from actual responses.

If the correct business catalog lacks an interface, sync once and inspect that same interface again. If it is still absent, give the administrator the system, environment and interface. A result from `bundled-cli`, including `profiles.delivery-center`, is not this evidence.

Run the original read. Report complete results or the exact unresolved login, permission or business choice.
