//! Trusted next actions, represented as argv/URLs rather than shell source.
use crate::{Runtime, profile, profiles};
use serde_json::{Value, json};

fn command(id: &str, argv: Vec<String>, reason: &str, interaction: bool) -> Value {
    json!({"id":id,"actor":"employee","kind":"command","argv":argv,"requiresInteraction":interaction,"reason":reason})
}
fn manual(actor: &str, reason: &str) -> Value {
    json!({"id":"manual-review","actor":actor,"kind":"manual","requiresInteraction":true,"reason":reason})
}
fn reauthorize(bound: &profiles::Binding) -> Value {
    let mut argv = vec![
        "dt-cli".into(),
        "auth".into(),
        "login".into(),
        "--profile".into(),
        bound.profile.clone(),
        "--environment".into(),
        bound.environment.clone(),
    ];
    if let Some(system) = bound.system_id.as_ref() {
        argv.extend(["--system".into(), system.clone()]);
    }
    command(
        "reauthorize",
        argv,
        "在交互终端由本人恢复此独立授权及所选同意范围",
        true,
    )
}
fn handle(value: &Value) -> Option<&str> {
    value.as_str().filter(|v| {
        v.len() == 43
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    })
}

pub(crate) fn attach(rt: &Runtime, value: &mut Value) {
    if value["ok"] != false {
        return;
    }
    let profile = value["profile"]
        .as_str()
        .or_else(|| {
            value["operationId"]
                .as_str()
                .filter(|op| op.starts_with("tasks."))
                .and_then(|_| value["meta"]["binding"]["profile"].as_str())
        })
        .filter(|name| profile::validate_name(name).is_ok());
    let binding = profile.and_then(|name| profiles::binding(rt, name).ok());
    let code = value["error"]["code"].as_str().unwrap_or("");
    let recovery = &value["meta"]["recovery"];
    let intent = handle(&recovery["intentId"]);
    let job = handle(&recovery["jobId"]);
    let mut actions = Vec::new();
    if matches!(
        code,
        "AUTH_REQUIRED"
            | "AUTHORIZATION_EXPIRED"
            | "AUTHORIZATION_REVOKED"
            | "AUTH_REFRESH_OUTCOME_UNKNOWN"
            | "CONSENT_REQUIRED"
    ) || (code == "SCOPE_DENIED" && recovery["reason"] == "consent-missing")
    {
        if let Some(bound) = binding.as_ref() {
            actions.push(reauthorize(bound));
        } else {
            actions.push(manual(
                "employee",
                "在交互终端对明确的账号、环境和系统执行setup或auth login",
            ));
        }
    } else if code == "SCOPE_DENIED" && recovery["reason"] == "catalog-outdated" {
        if let Some(bound) = binding.as_ref() {
            actions.push(command(
                "sync-contract",
                vec![
                    "dt-cli".into(),
                    "catalog".into(),
                    "sync".into(),
                    "--profile".into(),
                    bound.profile.clone(),
                ],
                "同步当前后台权限；仍不可用时由管理员核对赋权",
                false,
            ));
        }
    } else if code == "CONTRACT_CHANGED" {
        actions.push(
            if let Some(bound) = binding.as_ref().filter(|b| b.provider == "governed") {
                command(
                    "sync-contract",
                    vec![
                        "dt-cli".into(),
                        "catalog".into(),
                        "sync".into(),
                        "--profile".into(),
                        bound.profile.clone(),
                    ],
                    "同步后重新核对版本和后台权限，不自动改用其他版本",
                    false,
                )
            } else {
                manual("employee", "核对本次合同和客户端版本")
            },
        );
    } else if matches!(code, "GRANT_EXPIRED" | "SCOPE_DENIED" | "DATA_SCOPE_DENIED")
        && recovery["reason"] != "consent-missing"
    {
        actions.push(manual(
            "admin",
            "核对原操作的当前授权权限和数据范围；重新登录不会恢复管理员权限",
        ));
    } else if matches!(
        code,
        "LOCAL_STATE_UNAVAILABLE" | "CREDENTIAL_STORE_UNAVAILABLE" | "CREDENTIAL_DECODE_FAILED"
    ) {
        actions.push(manual(
            "operator",
            "恢复本地凭证目录或私有记录访问权限；保留已确认结果与原ID，不输出凭证或重发业务",
        ));
    } else if let Some(id) = handle(&recovery["runId"]) {
        actions.push(command(
            "inspect-run",
            vec!["dt-cli".into(), "tasks".into(), "status".into(), id.into()],
            "查询原执行记录，不重新派发",
            false,
        ));
    } else if let (Some(id), Some(name)) = (intent, profile) {
        actions.push(command(
            "inspect-intent",
            vec![
                "dt-cli".into(),
                "intents".into(),
                "status".into(),
                id.into(),
                "--profile".into(),
                name.into(),
            ],
            "查询原意图；结果未知时不得重新派发",
            false,
        ));
    } else if let (Some(id), Some(name)) = (job, profile) {
        actions.push(command(
            "inspect-job",
            vec![
                "dt-cli".into(),
                "jobs".into(),
                "status".into(),
                id.into(),
                "--profile".into(),
                name.into(),
            ],
            "查询原任务，不重新提交",
            false,
        ));
    } else {
        let action = match code {
            "PROFILE_REQUIRED" | "PROFILE_SELECTION_MISMATCH" | "PROFILE_PROVIDER_MISMATCH" => {
                command(
                    "select-profile",
                    vec!["dt-cli".into(), "profiles".into(), "list".into()],
                    "查看账号并显式选择匹配身份",
                    false,
                )
            }
            "PROFILE_NOT_CONFIGURED" => {
                if let Some(name) = profile
                    && rt.environments.len() == 1
                {
                    let mut argv = vec![
                        "dt-cli".into(),
                        "setup".into(),
                        "--profile".into(),
                        name.into(),
                        "--environment".into(),
                        rt.environments.keys().next().unwrap().clone(),
                    ];
                    if let Some(system) = value["meta"]["taskId"]
                        .as_str()
                        .and_then(crate::tasks::system_for)
                    {
                        argv.extend(["--system".into(), system.into()]);
                    }
                    command("setup-profile", argv, "为本次明确目标配置本人访问", true)
                } else {
                    manual("employee", "选择账号名称和已登记环境后执行setup")
                }
            }
            "AUTH_REQUIRED"
            | "AUTHORIZATION_EXPIRED"
            | "AUTHORIZATION_REVOKED"
            | "AUTH_REFRESH_OUTCOME_UNKNOWN"
            | "INTERACTION_REQUIRED" => {
                if let Some(bound) = binding.as_ref() {
                    reauthorize(bound)
                } else {
                    manual(
                        "employee",
                        "在交互终端对明确的账号、环境和系统执行setup或auth login",
                    )
                }
            }
            "SCOPE_DENIED" | "DATA_SCOPE_DENIED" => manual(
                "admin",
                "核对精确操作的本人同意、当前权限和业务数据范围；原因尚不完整，重复登录不会恢复管理员权限",
            ),
            "AUTH_DENIED" => manual(
                "employee",
                "本次授权未获确认，核对本人决定与权限；未据此推断授权到期",
            ),
            "CREDENTIAL_STORE_UNAVAILABLE" | "LOCAL_STATE_UNAVAILABLE" => manual(
                "operator",
                "恢复系统安全存储或本地私有记录的访问；不要改用明文凭证",
            ),
            "PARTIAL_RESULT" | "DATA_CHANGED" | "PAGINATION_UNKNOWN" | "RESULT_LIMIT" => manual(
                "employee",
                "保留已得到的数据和缺失范围，按原筛选重新查询；不要将失败当零条",
            ),
            "CLIENT_UPGRADE_REQUIRED" => command(
                "check-upgrade",
                vec!["dt-cli".into(), "upgrade".into(), "--check".into()],
                "核对安装并从组织可信入口取得兼容版本",
                false,
            ),
            "VERSION_REQUIRED" => manual(
                "employee",
                "从当前profile目录查看目标操作的候选版本，明确选择精确版本；不自动取最新",
            ),
            "UNKNOWN_TASK" => command(
                "find-task",
                vec!["dt-cli".into(), "tasks".into(), "list".into()],
                "查找已登记任务",
                false,
            ),
            "INVALID_ARGUMENT" | "UNKNOWN_OPERATION" => command(
                "inspect-schema",
                vec!["dt-cli".into(), "help".into()],
                "核对已登记命令和缺失参数",
                false,
            ),
            _ => manual(
                "operator",
                "保留错误代码与脱敏trace，核对当前环境；不自动重复业务写入",
            ),
        };
        actions.push(action);
    }
    if let Some(id) = intent
        && let Some(bound) = binding.as_ref()
        && let Ok(env) = rt.environment(&bound.environment)
    {
        let url = format!("{}/cli-governed-intent.html?intentId={id}", env.api_origin);
        actions.push(json!({"id":"reconcile-intent","actor":"employee","kind":"browser","url":url,"requiresInteraction":true,"reason":"在可信IAM页面核对原意图结果，核对不会重新发送"}));
    }
    value["meta"]["actions"] = json!(actions);
}

#[cfg(test)]
mod tests;
