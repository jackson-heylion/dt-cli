//! 恢复动作只绑定原本人、环境、用途和记录，不生成 Shell 或新的写入。
use super::{
    model::{PlatformProfile, SessionPurpose},
    read_profile_for,
};
use crate::{
    Runtime,
    interface::request::{Action, Platform},
    output::{Failure, PartialData, Recovery, RecoveryAction},
};
pub(super) fn bound(name: &str, p: &PlatformProfile) -> Recovery {
    Recovery {
        profile: Some(name.into()),
        environment: Some(p.environment.clone()),
        purpose: if p.purpose == SessionPurpose::Management {
            "management"
        } else {
            "business"
        }
        .into(),
        ..Default::default()
    }
}
fn command(
    id: &'static str,
    argv: Vec<String>,
    reason: &'static str,
    interaction: bool,
) -> RecoveryAction {
    RecoveryAction {
        id,
        argv,
        reason,
        requires_interaction: interaction,
    }
}
pub(super) async fn attach(rt: &Runtime, request: &Platform, error: &mut Failure) {
    if error
        .recovery
        .as_ref()
        .is_some_and(|r| !r.actions.is_empty())
    {
        return;
    }
    let job_id = error.partial_data.as_deref().and_then(|p| {
        if let PartialData::Job(summary) = p {
            Some(summary.job_id.clone())
        } else {
            None
        }
    });
    let existing = error.recovery.take();
    let mut recovery = if existing.is_some() {
        existing
    } else {
        match &request.action {
            Action::Writes(input) if !input.id.is_empty() => {
                super::writes::recovery(rt, &input.id).await.ok()
            }
            Action::Jobs(input) if !input.id.is_empty() || job_id.is_some() => {
                super::jobs::recovery(rt, job_id.as_deref().unwrap_or(&input.id))
                    .await
                    .ok()
            }
            _ => None,
        }
    };
    if recovery.as_ref().is_none_or(|r| r.profile.is_none()) {
        let purpose = if matches!(request.action, Action::Manage(_)) {
            SessionPurpose::Management
        } else {
            SessionPurpose::Cli
        };
        let name = match &request.action {
            Action::Manage(input) => input.profile.as_deref(),
            Action::Writes(input) | Action::Jobs(input) => input.profile.as_deref(),
            _ => request.profile.as_deref(),
        };
        if let Ok(name) = super::context::selected(rt, name, purpose) {
            if let Ok(profile) = read_profile_for(rt, &name, purpose) {
                let binding = bound(&name, &profile);
                recovery = Some(if let Some(mut original) = recovery.take() {
                    original.profile = binding.profile;
                    original.environment = binding.environment;
                    original.purpose = binding.purpose;
                    original
                } else {
                    binding
                });
            } else if let Action::Login(input) = &request.action {
                recovery = Some(Recovery {
                    profile: Some(name),
                    environment: Some(input.environment.clone()),
                    purpose: "business".into(),
                    ..Default::default()
                });
            }
        }
    }
    let mut recovery = recovery.unwrap_or_else(|| Recovery {
        purpose: if matches!(request.action, Action::Manage(_)) {
            "management"
        } else {
            "business"
        }
        .into(),
        ..Default::default()
    });
    if matches!(request.action, Action::Jobs(_)) && error.retryable {
        if let (Some(id), Some(name)) = (&recovery.record_id, &recovery.profile) {
            recovery.actions = vec![command(
                "resume-original-job",
                vec![
                    "dt-cli".into(),
                    "jobs".into(),
                    "resume".into(),
                    "--id".into(),
                    id.clone(),
                    format!("--profile={name}"),
                ],
                "沿原身份、版本、范围和剩余预算继续同一次读取工作包。",
                false,
            )];
        }
    }
    if let Action::Manage(input) = &request.action {
        if let Some(id) = &input.id
            && matches!(input.action, "inspect" | "apply" | "status" | "audit")
        {
            recovery.plan_id.get_or_insert_with(|| id.clone());
        }
        if let (Some(id), Some(name)) = (&recovery.plan_id, &recovery.profile) {
            recovery.actions.push(command(
                "inspect-original-plan",
                vec![
                    "dt-cli".into(),
                    "manage".into(),
                    "status".into(),
                    "--id".into(),
                    id.clone(),
                    format!("--profile={name}"),
                ],
                "仅回读原计划与执行结果，不重新接入或扩大授权。",
                false,
            ));
        }
    }
    if matches!(
        error.code,
        "AUTH_REQUIRED" | "PLATFORM_CONTEXT_CHANGED" | "PROFILE_NOT_CONFIGURED"
    ) {
        if let (Some(name), Some(environment)) = (&recovery.profile, &recovery.environment) {
            let mut argv = vec!["dt-cli".into()];
            if recovery.purpose == "management" {
                argv.push("manage".into());
            }
            argv.extend([
                "login".into(),
                format!("--profile={name}"),
                "--environment".into(),
                environment.clone(),
            ]);
            recovery.actions.insert(
                0,
                command(
                    "login-original-purpose",
                    argv,
                    "沿用原本人、环境和用途验证；写入确认不会因此复活。",
                    true,
                ),
            );
        }
    }
    if recovery.actions.is_empty() {
        recovery.actions.push(command(
            "inspect-command",
            vec!["dt-cli".into(), "help".into()],
            "核对命令与失败范围，不自动重复业务写入。",
            false,
        ));
    }
    error.recovery = Some(recovery);
}
