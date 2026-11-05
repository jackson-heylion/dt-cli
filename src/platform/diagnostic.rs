//! 诊断默认只查元数据；明确只读范围才发一次 Gateway 请求。
use super::{catalog, contract::*, discovery, model::PlatformProfile, session, transport};
use crate::{
    Runtime,
    output::{Failure, Result},
};
use clap::{Arg, ArgMatches, Command};
use serde::Serialize;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Check {
    /// 检查项目名称。
    item: &'static str,
    /// VERIFIED、FAILED、NOT_EXECUTED 或 DEPENDENCY_MISSING。
    state: &'static str,
    /// 稳定安全结果代码。
    code: String,
    /// 恢复动作。
    next_action: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    /// 本机实际 CLI 版本，不等同已发行。
    cli_version: &'static str,
    /// 当前明确本人及环境。
    identity: Option<super::model::SessionView>,
    /// 已固定授权接口，可空。
    published: Option<Published>,
    /// 本次可信正式发行信息，无法核验为空。
    stable_release: Option<crate::release::StableDiagnostic>,
    /// 本人当前合法视角和最近选择提示。
    views: Vec<ViewOption>,
    /// 各项独立核验状态。
    checks: Vec<Check>,
    /// 明确 READ 探测结果，可空；最多一次请求。
    probe: Option<super::executor::ExecutionResult>,
}
pub(super) fn command() -> Command {
    discovery::target(Command::new("diagnose").arg(Arg::new("profile").long("profile")))
        .arg(
            Arg::new("probe")
                .long("probe")
                .action(clap::ArgAction::SetTrue)
                .requires_all(["input-file", "scope"]),
        )
        .arg(Arg::new("input-file").long("input-file").requires("probe"))
        .arg(
            Arg::new("body-file")
                .long("body-file")
                .requires("input-file"),
        )
        .arg(Arg::new("scope").long("scope").requires("probe"))
}
fn checked(
    item: &'static str,
    state: &'static str,
    code: impl Into<String>,
    next_action: &'static str,
) -> Check {
    Check {
        item,
        state,
        code: code.into(),
        next_action,
    }
}
fn failed(item: &'static str, e: &Failure) -> Check {
    checked(
        item,
        if e.retryable {
            "DEPENDENCY_MISSING"
        } else {
            "FAILED"
        },
        e.code,
        "CORRECT_REPORTED_DEPENDENCY",
    )
}
pub(super) async fn execute(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &ArgMatches,
) -> Result<serde_json::Value> {
    let mut report = Report {
        cli_version: env!("CARGO_PKG_VERSION"),
        identity: None,
        published: None,
        stable_release: None,
        views: vec![],
        checks: vec![],
        probe: None,
    };
    match session::me(rt, p).await {
        Ok(v) => {
            report.identity = Some(v);
            report
                .checks
                .push(checked("identity", "VERIFIED", "CURRENT_SESSION", "NONE"));
        }
        Err(e) => report.checks.push(failed("identity", &e)),
    };
    match discovery::for_diagnostic(rt, p, o).await {
        Ok(v) => {
            report.checks.push(checked(
                "authorization",
                "VERIFIED",
                "FIXED_AUTHORIZED_VERSION",
                "NONE",
            ));
            report.published = Some(v);
        }
        Err(e) => {
            report.checks.push(failed("authorization", &e));
            report.checks.push(checked(
                "views",
                "NOT_EXECUTED",
                "CONTRACT_UNAVAILABLE",
                "RESOLVE_AUTHORIZED_INTERFACE",
            ));
        }
    }
    if let Some(v) = &report.published {
        match catalog::verify(p, v, &v.contract.operation_id, v.contract.operation_version) {
            Ok(()) => report.checks.push(checked(
                "interface-minimum-version",
                "VERIFIED",
                "CLI_COMPATIBLE",
                "NONE",
            )),
            Err(e) => report.checks.push(failed("interface-minimum-version", &e)),
        }
    } else {
        report.checks.push(checked(
            "interface-minimum-version",
            "NOT_EXECUTED",
            "CONTRACT_UNAVAILABLE",
            "RESOLVE_AUTHORIZED_INTERFACE",
        ));
    }
    let minimum = report
        .published
        .as_ref()
        .map(|v| v.contract.minimum_cli_version.as_str())
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    match crate::release::stable_diagnostic(minimum).await {
        Ok(v) => {
            report.checks.push(checked(
                "stable-release",
                if v.minimum_satisfied {
                    "VERIFIED"
                } else {
                    "FAILED"
                },
                if v.minimum_satisfied {
                    "TRUSTED_STABLE"
                } else {
                    "STABLE_VERSION_INSUFFICIENT"
                },
                "VERIFY_INSTALLABLE_RELEASE",
            ));
            report.stable_release = Some(v);
        }
        Err(e) => report.checks.push(failed("stable-release", &e)),
    };
    report.checks.push(checked(
        "local-version",
        if cfg!(feature = "local-dev") {
            "NOT_EXECUTED"
        } else {
            "VERIFIED"
        },
        if cfg!(feature = "local-dev") {
            "LOCAL_DEVELOPMENT_BINARY"
        } else {
            "RUNNING_BINARY_VERSION"
        },
        "VERIFY_INSTALLABLE_RELEASE",
    ));
    if let Some(v) = &report.published {
        if v.contract.view_types.is_empty() {
            report
                .checks
                .push(checked("views", "VERIFIED", "VIEW_NOT_REQUIRED", "NONE"));
        } else {
            match catalog::views_for(rt, p, &v.contract).await {
                Ok(views) => {
                    let n = views.iter().filter(|v| v.supported).count();
                    report.checks.push(checked(
                        "views",
                        if n > 0 { "VERIFIED" } else { "FAILED" },
                        if n == 0 {
                            "NO_LEGAL_VIEW"
                        } else if n == 1 {
                            "UNIQUE_VIEW"
                        } else {
                            "VIEW_SELECTION_REQUIRED"
                        },
                        if n == 1 {
                            "NONE"
                        } else {
                            "SELECT_EXPLICIT_VIEW"
                        },
                    ));
                    report.views = views;
                }
                Err(e) => report.checks.push(failed("views", &e)),
            }
        }
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Route {
        /// 平台部署方固定的 Gateway origin，不是上游地址。
        gateway_base_url: String,
    }
    let route = async {
        let credentials = session::access(rt, p).await?;
        let route: Route = transport::request(
            transport::client()?
                .get(format!("{}/platform/cli/catalog/runtime", p.origin))
                .bearer_auth(&credentials.access_token),
        )
        .await?;
        transport::origin(&route.gateway_base_url)?;
        super::metadata::remember_runtime(p, route.gateway_base_url);
        Ok::<(), Failure>(())
    }
    .await;
    match route {
        Ok(()) => {
            report.checks.push(checked(
                "route-configuration",
                "VERIFIED",
                "GATEWAY_CONFIGURED",
                "NONE",
            ));
            report.checks.push(checked(
                "route-reachability",
                "NOT_EXECUTED",
                "NO_BUSINESS_REQUEST",
                "USE_EXPLICIT_READ_PROBE",
            ));
        }
        Err(e) => report.checks.push(failed("route-configuration", &e)),
    };
    if o.get_flag("probe") {
        let v = report.published.as_ref().ok_or_else(|| {
            Failure::new(
                "PROBE_INTERFACE_REQUIRED",
                2,
                "无法确定获准固定接口，未发业务请求。",
            )
        })?;
        catalog::verify(p, v, &v.contract.operation_id, v.contract.operation_version)?;
        if v.contract.effect != Effect::Read {
            return Err(Failure::new(
                "READ_PROBE_REQUIRED",
                4,
                "诊断只允许明确的 READ 接口，未发业务请求。",
            ));
        }
        let scope = discovery::text(o, "scope");
        if scope.is_empty() || scope != v.contract.data_scope {
            return Err(Failure::new(
                "PROBE_SCOPE_MISMATCH",
                2,
                "探测范围必须与固定契约明确范围相同，未发业务请求。",
            ));
        }
        let input = super::invocation::input_file(
            discovery::text(o, "input-file"),
            o.get_one::<String>("body-file").map(String::as_str),
        )?;
        // encode 校验完整参数，再核验本次视角；此处不会访问业务。
        let mut wire = super::invocation::encode(&v.contract, &input)?;
        wire.view = super::executor::choose_view(rt, p, &v.contract, wire.view).await?;
        match super::executor::execute(rt, p, v, &wire, None, v.contract.limits.max_seconds.into())
            .await
        {
            Ok(result) => {
                report.checks.push(checked(
                    "probe",
                    if result.business_success {
                        "VERIFIED"
                    } else {
                        "FAILED"
                    },
                    "ONE_READ_REQUEST",
                    "INSPECT_PROBE_RESULT",
                ));
                report.probe = Some(result);
            }
            Err(e) => report.checks.push(failed("probe", &e)),
        }
    } else {
        report.checks.push(checked(
            "probe",
            "NOT_EXECUTED",
            "ZERO_BUSINESS_REQUESTS",
            "NONE",
        ));
    }
    super::serialize(report)
}
