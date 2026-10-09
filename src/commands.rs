use crate::{
    Runtime,
    catalog::{Catalog, Operation, flag, string},
    governed, login, open,
    output::{self, Executed, Failure, Result, invalid},
    parameters::{Prepared, api_parameters, prepare},
    profile, release,
};
use serde_json::{Map, Value, json};
pub async fn execute(rt: &Runtime, args: Vec<String>) -> (Value, u8, bool) {
    let (mut value, code, table) = execute_inner(rt, args).await;
    crate::recovery::attach(rt, &mut value);
    (value, code, table)
}

async fn execute_inner(rt: &Runtime, args: Vec<String>) -> (Value, u8, bool) {
    let catalog = Catalog::shared();
    let m = match catalog.invocation_command(&args).try_get_matches_from(args) {
        Ok(m) => m,
        Err(_) => {
            let (v, c) = output::envelope_result("cli", None, Err(invalid()));
            return (v, c, false);
        }
    };
    let table = string(&m, "format") == Some("table");
    if let Some((group, group_matches)) = m.subcommand()
        && group_matches.subcommand().is_none()
        && flag(group_matches, "help")
        && catalog
            .operations
            .iter()
            .any(|op| op.command.starts_with(&format!("{group} ")))
    {
        let mut help = catalog.help();
        help["commands"].as_array_mut().unwrap().retain(|item| {
            item["command"]
                .as_str()
                .is_some_and(|command| command.starts_with(&format!("{group} ")))
        });
        let (value, code) = output::envelope_result("help", None, Ok(help));
        return (value, code, table);
    }

    let (op, leaf) = match catalog.resolve(&m) {
        Ok(v) => v,
        Err(e) => {
            let (v, c) = output::envelope_result("cli", None, Err(e));
            return (v, c, table);
        }
    };
    let name = string(leaf, "profile");
    // Help depends only on the release catalog, even with an absent or damaged profile.
    if flag(leaf, "help") || flag(&m, "help") {
        let result = if op.operation_id == "help" {
            Ok(catalog.help())
        } else {
            catalog.schema(&op.operation_id)
        };
        let (v, c) = output::envelope_result(&op.operation_id, name, result);
        return (v, c, table);
    }
    if matches!(
        op.operation_id.as_str(),
        "setup" | "profiles.list" | "profiles.use" | "profiles.delivery-center"
    ) {
        let result = match op.operation_id.as_str() {
            "profiles.list" => crate::profiles::list(rt),
            "profiles.delivery-center" => crate::governed::set_delivery_center(
                rt,
                name.unwrap(),
                string(leaf, "id").unwrap().parse::<i32>().unwrap_or(0),
            ),
            "profiles.use" => crate::profiles::select(rt, string(leaf, "name").unwrap()),
            _ => {
                crate::profiles::setup(
                    rt,
                    name.unwrap(),
                    string(leaf, "environment").unwrap(),
                    string(leaf, "system"),
                )
                .await
            }
        };
        let (value, code) = output::envelope_result(&op.operation_id, name, result);
        return (value, code, table);
    }
    if op.operation_id.starts_with("tasks.") {
        let result = crate::tasks::dispatch(rt, op, leaf).await;
        let (mut value, code) = output::envelope(
            &op.operation_id,
            name,
            match result {
                Ok(v) => Executed::value(v),
                Err(e) => Executed::failed(e),
            },
        );
        crate::tasks::annotate(&mut value, string(leaf, "task"));
        return (value, code, table);
    }
    let governed = match name {
        Some(name) => governed::profile_exists(&rt.root, name),
        None => Ok(false),
    };
    let governed = match governed {
        Ok(value) => value,
        Err(failure) => {
            let (v, c) = output::envelope_result(&op.operation_id, name, Err(failure));
            return (v, c, table);
        }
    };
    if op.operation_id == "auth.login"
        && let Err(failure) = login::validate_method(string(leaf, "login-method"))
    {
        let (value, code) = output::envelope_result(&op.operation_id, name, Err(failure));
        return (value, code, table);
    }
    // A governed profile, `auth login --system` or a governed-only command never falls back to
    // the personal-workflow provider.
    if governed
        || (op.operation_id == "auth.login" && string(leaf, "system").is_some())
        || op.operation_id.starts_with("jobs.")
        || op.operation_id.starts_with("intents.")
        || op.operation_id == "catalog.sync"
    {
        let result = governed::dispatch(rt, op, leaf).await;
        let (v, c) = output::envelope(
            &op.operation_id,
            name,
            match result {
                Ok(value) => Executed::value(value),
                Err(failure) => Executed::failed(failure),
            },
        );
        return (v, c, table);
    }
    let id = if op.operation_id == "api.call" {
        string(leaf, "operation")
            .and_then(|id| catalog.operation(id).ok())
            .filter(|operation| operation.route.is_some())
            .map(|operation| operation.operation_id.as_str())
            .unwrap_or(&op.operation_id)
    } else {
        &op.operation_id
    };
    // Parameter validation happens here, before any profile, credential or network access.
    let prepared = if flag(leaf, "help") || flag(&m, "help") {
        Prepared::none()
    } else {
        match prepare(catalog, op, leaf) {
            Ok(prepared) => prepared,
            Err(e) => {
                let (v, c) = output::envelope_result(id, name, Err(e));
                return (v, c, table);
            }
        }
    };
    let bounded = prepared.plan.as_ref().is_some_and(|plan| plan.all);
    let result = if op.operation_id == "auth.login" || bounded {
        // Cancellation and the aggregation deadline are owned by the bounded loop itself,
        // so the partial result survives an interrupt.
        dispatch(rt, catalog, op, leaf, &prepared, flag(&m, "help")).await
    } else {
        tokio::select! {
            r = dispatch(rt, catalog, op, leaf, &prepared, flag(&m,"help")) => r,
            _ = tokio::signal::ctrl_c() => Err(Failure::new("LOGIN_CANCELLED",8,"操作已取消。")),
        }
    };
    let (v, c) = output::envelope(
        id,
        name,
        match result {
            Ok(executed) => executed,
            Err(failure) => Executed::failed(failure),
        },
    );
    (v, c, table)
}

async fn dispatch(
    rt: &Runtime,
    catalog: &Catalog,
    op: &Operation,
    leaf: &clap::ArgMatches,
    prepared: &Prepared,
    root_help: bool,
) -> Result<Executed> {
    let name = string(leaf, "profile");

    if root_help || flag(leaf, "help") {
        return match op.operation_id.as_str() {
            "help" => Ok(catalog.help()),
            _ => catalog.schema(&op.operation_id),
        }
        .map(Executed::value);
    }
    let selected = string(leaf, "operation");
    if matches!(op.operation_id.as_str(), "auth.check" | "doctor")
        && let Some(id) = selected
    {
        let target = catalog.operation(id)?;
        if target.safe_probe.as_deref() != Some("iam.profile.get") {
            return Ok(Executed::value(
                json!({"onlineVerified":false,"reason":"no-safe-probe"}),
            ));
        }
    }
    if flag(leaf, "dry-run") {
        if let Some(name) = name {
            profile::validate_name(name)?;
        }
        let effect = if op.operation_id == "api.call" {
            catalog.operation(selected.unwrap())?.effect.clone()
        } else {
            op.effect.clone()
        };
        let mut preview = json!({"preview":true,"operationId":if op.operation_id=="api.call"{selected.unwrap()}else{&op.operation_id},"effect":effect,"environment":"unresolved","authentication":"unresolved","httpRequests":0,"credentialReads":0,"browserOpened":false});
        if let Some(plan) = prepared.plan.as_ref() {
            preview["mode"] = json!(if plan.all { "all" } else { "page" });
            preview["maxPages"] = json!(plan.limits.max_pages);
            preview["maxItems"] = json!(plan.limits.max_items);
            preview["pageSize"] = json!(plan.limits.page_size);
            preview["estimatedRequests"] = json!(if plan.all { plan.limits.max_pages } else { 1 });
        }
        return Ok(Executed::value(preview));
    }
    // The bounded aggregation owns its own cancellation and deadline handling.
    if let Some(plan) = prepared.plan.as_ref()
        && plan.all
    {
        return rt
            .online_all(name.ok_or_else(invalid)?, plan, &plan.params)
            .await;
    }
    let result: Result<Value> = match op.operation_id.as_str() {
        "help" => {
            if let Some(id) = selected {
                catalog.schema(id)
            } else {
                Ok(catalog.help())
            }
        }
        "version" => Ok(release::build_info()),
        "install" | "upgrade" => release::execute(&op.operation_id, leaf).await,
        "schema" => {
            let mut s = catalog.schema(selected.ok_or_else(invalid)?)?;
            if flag(leaf, "compact") {
                s.as_object_mut().unwrap().remove("examples");
            }
            Ok(s)
        }
        "discover" => {
            let query = string(leaf, "query").unwrap_or("").to_lowercase();
            let limit = string(leaf, "limit")
                .unwrap()
                .parse::<usize>()
                .map_err(|_| invalid())?;
            Ok(catalog.discover(&query, limit))
        }
        "auth.login" => {
            return login::login_with_method(
                rt,
                name.ok_or_else(invalid)?,
                string(leaf, "environment"),
                string(leaf, "login-method"),
            )
            .await
            .map(Executed::value);
        }
        "auth.status" => {
            let p = profile::read(&rt.root, name.ok_or_else(invalid)?)?
                .ok_or_else(|| Failure::new("PROFILE_NOT_CONFIGURED", 2, "profile 尚未配置。"))?;
            rt.environment(&p.environment)?;
            let credentials = rt.store.read(&p.credential_key)?;
            let present = credentials.is_some();
            let access_expires_at = credentials
                .as_ref()
                .and_then(|record| record.access_expires_at.as_ref())
                .unwrap_or(&p.access_expires_at);
            let authorization_expires_at = credentials
                .as_ref()
                .and_then(|record| record.authorization_expires_at.as_ref())
                .unwrap_or(&p.authorization_expires_at);
            Ok(
                json!({"environment":p.environment,"subjectId":p.subject_id,"authorizationId":p.authorization_id,"credentialPresent":present,"accessExpiresAt":access_expires_at,"authorizationExpiresAt":authorization_expires_at,"lastCheckedAt":p.checked_at,"checkSource":"local-record","onlineVerified":false}),
            )
        }
        "auth.logout" => rt.logout(name.ok_or_else(invalid)?).await,
        "iam.profile.get" => rt.check(name.ok_or_else(invalid)?).await,
        "iam.apps.list" => {
            rt.online(name.ok_or_else(invalid)?, "iam.apps.list", json!({}))
                .await
        }
        "iam.workflow.applications" | "iam.workflow.form-types" => {
            let mut params = Map::new();
            for p in &op.parameters {
                if p.name != "profile"
                    && p.name != "dry-run"
                    && let Some(v) = string(leaf, &p.name)
                {
                    params.insert(
                        p.name.replace('-', "_"),
                        if p.kind == "integer" {
                            json!(v.parse::<i64>().map_err(|_| invalid())?)
                        } else {
                            json!(v)
                        },
                    );
                }
            }
            for (a, b) in [("application_id", "applicationId")] {
                if let Some(v) = params.remove(a) {
                    params.insert(b.into(), v);
                }
            }
            rt.online(
                name.ok_or_else(invalid)?,
                &op.operation_id,
                Value::Object(params),
            )
            .await
        }
        "iam.workflow.open" => {
            let id = prepared.open_id.as_deref().ok_or_else(invalid)?;
            return open::open(rt, name.ok_or_else(invalid)?, id)
                .await
                .map(Executed::value);
        }
        "iam.workflow.list" => {
            let plan = prepared.plan.as_ref().ok_or_else(invalid)?;
            rt.online_page(
                name.ok_or_else(invalid)?,
                plan.page,
                plan.limits.page_size,
                plan.params.clone(),
            )
            .await
        }
        "api.call" => {
            let id = selected.ok_or_else(invalid)?;
            let raw = prepared.api_raw.clone().unwrap_or_else(|| "{}".to_owned());
            let params = match prepared.api.clone() {
                Some(params) => params,
                None => api_parameters(catalog, id, &raw)?,
            };
            let profile = name.ok_or_else(invalid)?;
            if let Some(open_id) = prepared.open_id.as_deref() {
                // The generic channel goes through the same sensitive-result adapter.
                return open::open(rt, profile, open_id).await.map(Executed::value);
            }
            if let Some(plan) = prepared.plan.as_ref() {
                return rt
                    .online_page(
                        profile,
                        plan.page,
                        plan.limits.page_size,
                        plan.params.clone(),
                    )
                    .await
                    .map(Executed::value);
            }
            rt.online(profile, id, params).await
        }
        "auth.check" => rt.check(name.ok_or_else(invalid)?).await,
        "doctor" => {
            if flag(leaf, "online") {
                rt.check(name.ok_or_else(invalid)?).await
            } else {
                Ok(
                    json!({"onlineVerified":false,"credentialStore":"not-probed","environments":rt.environments.keys().collect::<Vec<_>>(),"catalogDigest":catalog.digest()}),
                )
            }
        }
        _ => Err(invalid()),
    };
    result.map(Executed::value)
}
