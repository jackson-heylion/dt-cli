use super::*;

pub(super) fn version_triple(value: &str) -> Option<(u64, u64, u64)> {
    if value.is_empty()
        || value.len() > 40
        || !value.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        return None;
    }
    let mut parts = value.split('.').map(|part| part.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}
pub(super) fn cli_version() -> (u64, u64, u64) {
    let release = env!("CARGO_PKG_VERSION").split(['-', '+']).next();
    release.and_then(version_triple).unwrap_or((0, 0, 0))
}
/// Whether this build may parse and execute the operation contract.
pub(super) fn compatible(operation: &Value) -> Result<()> {
    let minimum = operation["minimumCliVersion"]
        .as_str()
        .and_then(version_triple)
        .ok_or_else(catalog_invalid)?;
    if operation["contractSchemaVersion"].as_u64() != Some(CONTRACT_SCHEMA_VERSION)
        || minimum > cli_version()
    {
        return Err(upgrade_required());
    }
    Ok(())
}
pub(super) fn no_references(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            !["$ref", "$dynamicRef", "$recursiveRef"]
                .iter()
                .any(|key| map.contains_key(*key))
                && map.values().all(no_references)
        }
        Value::Array(items) => items.iter().all(no_references),
        _ => true,
    }
}
pub(super) fn digest_like(value: &Value) -> bool {
    value.as_str().is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
/// Structural checks for every operation. Contracts in a newer structure stay listed but are
/// only executable by a CLI that understands them, so their schemas are not compiled here.
pub(super) fn validate_operations(operations: &[Value]) -> Result<Vec<Option<ContractValidators>>> {
    let mut seen = BTreeSet::new();
    let mut validators = Vec::with_capacity(operations.len());
    for operation in operations {
        let fields = operation.as_object().ok_or_else(catalog_invalid)?;
        let id = operation["operationId"].as_str().filter(|v| valid_id(v));
        let version = operation["operationVersion"]
            .as_str()
            .filter(|v| version_triple(v).is_some());
        let (Some(id), Some(version)) = (id, version) else {
            return Err(catalog_invalid());
        };
        let schema_version = operation["contractSchemaVersion"]
            .as_u64()
            .filter(|v| *v >= 1);
        if schema_version.is_none()
            || !digest_like(&operation["contractDigest"])
            || !["read", "write"].contains(&operation["effect"].as_str().unwrap_or(""))
            || !operation["consented"].is_boolean()
            || operation["minimumCliVersion"]
                .as_str()
                .and_then(version_triple)
                .is_none()
            || ["target", "route", "binding", "host", "url", "headers"]
                .iter()
                .any(|key| fields.contains_key(*key))
            || !seen.insert((id.to_owned(), version.to_owned()))
        {
            return Err(catalog_invalid());
        }
        if schema_version == Some(CONTRACT_SCHEMA_VERSION) {
            let compile = |key: &str| {
                let schema = operation
                    .get(key)
                    .filter(|schema| schema.is_object())
                    .ok_or_else(catalog_invalid)?;
                if !no_references(schema) {
                    return Err(catalog_invalid());
                }
                jsonschema::draft202012::new(schema)
                    .map(std::sync::Arc::new)
                    .map_err(|_| catalog_invalid())
            };
            validators.push(Some(ContractValidators {
                input: compile("inputSchema")?,
                output: compile("outputSchema")?,
            }));
        } else {
            validators.push(None);
        }
    }
    Ok(validators)
}

pub(super) fn cache_from_data(data: &Value, p: &GovernedProfile) -> Result<Cache> {
    let operations = data["operations"]
        .as_array()
        .ok_or_else(http::protocol)?
        .clone();
    let validators = validate_operations(&operations)?;
    let revision = data["catalogRevision"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 64);
    let cached_at = data["cachedAt"].as_str().filter(|v| rfc3339(v).is_some());
    let (Some(revision), Some(cached_at)) = (revision, cached_at) else {
        return Err(http::protocol());
    };
    if data["systemId"] != p.system_id
        || data["environment"] != p.environment
        || data["authorizationId"] != p.authorization_id
        || data["subject"]["id"] != p.subject_id
        || data["authorizationExpiresAt"] != p.authorization_expires_at
    {
        return Err(identity_mismatch());
    }
    Ok(Cache {
        provider: PROVIDER.into(),
        issuer: p.issuer.clone(),
        environment: p.environment.clone(),
        system_id: p.system_id.clone(),
        subject_id: p.subject_id.clone(),
        authorization_id: p.authorization_id.clone(),
        catalog_revision: revision.into(),
        cached_at: cached_at.into(),
        operations,
        validators,
    })
}
pub(super) async fn live_catalog(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
) -> Result<Cache> {
    let reply = authorized(rt, p, env, client, |token| {
        Ok(client.get(endpoint(env, CATALOG_ROUTE)?).bearer_auth(token))
    })
    .await?;
    cache_from_data(&envelope(reply)?["data"], p)
}
pub(super) fn consented(cache: &Cache) -> usize {
    cache
        .operations
        .iter()
        .filter(|operation| operation["consented"] == true)
        .count()
}

pub(super) fn select<'a>(cache: &'a Cache, id: &str, version: Option<&str>) -> Result<&'a Value> {
    let matches: Vec<_> = cache
        .operations
        .iter()
        .filter(|item| {
            item["operationId"] == id && version.is_none_or(|v| item["operationVersion"] == v)
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(Failure::new(
            "UNKNOWN_OPERATION",
            6,
            "受控目录中没有该操作版本。",
        )),
        _ => Err(Failure::new(
            "VERSION_REQUIRED",
            2,
            "该操作有多个版本，请用 --version 指定。",
        )),
    }
}
/// Local gates in order: a contract this build understands, cached backend permission, the effect
/// this command executes (reads by `api call`/`jobs`, writes only through intents).
pub(super) fn usable(operation: &Value, effect: &str) -> Result<()> {
    compatible(operation)?;
    if operation["consented"] != true {
        let mut error = Failure::new(
            "SCOPE_DENIED",
            4,
            "缓存未确认此接口版本的后台权限；请 catalog sync 后重试，仍不足时联系管理员。",
        );
        error.recovery = Some(Box::new(json!({"reason":"catalog-outdated"})));
        return Err(error);
    }
    if operation["effect"] != effect {
        return Err(if effect == "read" {
            Failure::new(
                "SCOPE_DENIED",
                4,
                "写入操作必须经可信确认流程：请用 dt-cli intents prepare。",
            )
        } else {
            Failure::new("SCOPE_DENIED", 4, "读取操作请用 dt-cli api call。")
        });
    }
    Ok(())
}

pub(super) fn contract_key(operation: &Value) -> Result<String> {
    Ok(format!(
        "{}@{}#{}",
        operation["operationId"].as_str().ok_or_else(invalid)?,
        operation["operationVersion"].as_str().ok_or_else(invalid)?,
        operation["contractDigest"].as_str().ok_or_else(invalid)?
    ))
}
pub(super) fn validate_params(raw: &str, validator: &jsonschema::Validator) -> Result<Value> {
    if raw.len() > PARAMS_LIMIT {
        return Err(invalid());
    }
    let value = crate::input::strict_json(raw)?;
    if !value.is_object() {
        return Err(invalid());
    }
    if !validator.is_valid(&value) {
        return Err(invalid());
    }
    Ok(value)
}
pub(super) fn status(rt: &Runtime, name: &str) -> Result<Value> {
    let p = require_profile(rt, name)?;
    rt.environment(&p.environment)?;
    let credentials = rt.store.read(&p.credential_key)?;
    let access_expires_at = credentials
        .as_ref()
        .and_then(|c| c.access_expires_at.clone())
        .unwrap_or_else(|| p.access_expires_at.clone());
    Ok(json!({
        "provider": PROVIDER,
        "environment": p.environment,
        "systemId": p.system_id,
        "subjectId": p.subject_id,
        "authorizationId": p.authorization_id,
        "credentialPresent": credentials.is_some(),
        "accessExpiresAt": access_expires_at,
        "authorizationExpiresAt": p.authorization_expires_at,
        "checkSource": "local-record",
        "permissionVerified": false,
        "onlineVerified": false,
    }))
}
pub(super) async fn check(rt: &Runtime, name: &str) -> Result<Value> {
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let live = live_catalog(rt, &p, env, &http::client()?).await?;
    Ok(json!({
        "provider": PROVIDER,
        "environment": p.environment,
        "systemId": p.system_id,
        "subjectId": p.subject_id,
        "authorizationId": p.authorization_id,
        "authorizationExpiresAt": p.authorization_expires_at,
        "catalogRevision": live.catalog_revision,
        "operationCount": live.operations.len(),
        "consentedCount": consented(&live),
        "permissionVerified": true,
        "onlineVerified": true,
    }))
}
pub(super) async fn sync(rt: &Runtime, name: &str) -> Result<Value> {
    let _lock = profile::lock(&rt.root, name).await?;
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let live = live_catalog(rt, &p, env, &http::client()?).await?;
    save_private(&cache_path(&rt.root, name)?, &live)?;
    Ok(json!({
        "provider": PROVIDER,
        "systemId": p.system_id,
        "environment": p.environment,
        "authorizationId": p.authorization_id,
        "catalogRevision": live.catalog_revision,
        "operationCount": live.operations.len(),
        "consentedCount": consented(&live),
        "cachedAt": live.cached_at,
        "permissionVerified": true,
        "onlineVerified": true,
    }))
}
/// Offline view of one cached operation, marked with whether this build can execute it.
pub(super) fn listed(operation: &Value) -> Value {
    let mut value = operation.clone();
    value["cliCompatible"] = json!(compatible(operation).is_ok());
    value
}
pub(super) fn discover(
    rt: &Runtime,
    name: &str,
    query: Option<&str>,
    limit: Option<&str>,
) -> Result<Value> {
    let limit = limit
        .unwrap_or("5")
        .parse::<usize>()
        .ok()
        .filter(|v| (1..=100).contains(v))
        .ok_or_else(invalid)?;
    let p = require_profile(rt, name)?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let query = query.unwrap_or("").to_lowercase();
    let operations: Vec<Value> = cache
        .operations
        .iter()
        .filter(|item| {
            format!(
                "{} {}",
                item["operationId"].as_str().unwrap_or(""),
                item["summary"].as_str().unwrap_or("")
            )
            .to_lowercase()
            .contains(&query)
        })
        .take(limit)
        .map(|operation| {
            let mut summary = listed(operation);
            for field in ["inputSchema", "outputSchema", "confirmation"] {
                summary.as_object_mut().unwrap().remove(field);
            }
            summary
        })
        .collect();
    Ok(json!({
        "provider": PROVIDER,
        "systemId": p.system_id,
        "environment": p.environment,
        "authorizationId": p.authorization_id,
        "catalogRevision": cache.catalog_revision,
        "cachedAt": cache.cached_at,
        "permissionVerified": false,
        "onlineVerified": false,
        "operations": operations,
    }))
}
pub(super) fn schema(rt: &Runtime, name: &str, id: &str, version: Option<&str>) -> Result<Value> {
    let p = require_profile(rt, name)?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let selected = select(&cache, id, version)?;
    Ok(json!({
        "provider": PROVIDER,
        "systemId": p.system_id,
        "environment": p.environment,
        "cachedAt": cache.cached_at,
        "permissionVerified": false,
        "onlineVerified": false,
        "schema": listed(selected),
    }))
}
