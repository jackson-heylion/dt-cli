//! Offline account selection for task commands. Existing commands keep explicit profiles.
use crate::{
    Runtime, governed, login,
    output::{Failure, Result, invalid},
    private_store, profile,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Binding {
    pub profile: String,
    pub provider: String,
    pub issuer: String,
    pub environment: String,
    pub system_id: Option<String>,
    pub subject_id: String,
    pub authorization_id: String,
    pub authorization_expires_at: String,
}

impl Binding {
    fn same_identity(&self, other: &Self) -> bool {
        self.profile == other.profile
            && self.provider == other.provider
            && self.issuer == other.issuer
            && self.environment == other.environment
            && self.system_id == other.system_id
            && self.subject_id == other.subject_id
    }
    pub(crate) fn matches(&self, provider: &str, system: Option<&str>) -> bool {
        self.provider == provider && self.system_id.as_deref() == system
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    schema_version: u32,
    binding: Binding,
}

fn selection_path(root: &Path) -> std::path::PathBuf {
    root.join("context").join("selection.json")
}

pub(crate) fn binding(rt: &Runtime, name: &str) -> Result<Binding> {
    profile::validate_name(name)?;
    let personal = profile::read(&rt.root, name)?;
    if governed::profile_exists(&rt.root, name)? {
        if personal.is_some() {
            return Err(Failure::new(
                "PROFILE_PROVIDER_MISMATCH",
                2,
                "同名profile存在两个身份来源；请明确处理冲突。",
            ));
        }
        return governed::binding(rt, name);
    }
    let p = personal.ok_or_else(|| {
        Failure::new(
            "PROFILE_NOT_CONFIGURED",
            2,
            "profile尚未配置；请先执行setup。",
        )
    })?;
    let env = rt.environment(&p.environment)?;
    Ok(Binding {
        profile: name.into(),
        provider: "personal".into(),
        issuer: env.api_origin.clone(),
        environment: p.environment,
        system_id: None,
        subject_id: p.subject_id,
        authorization_id: p.authorization_id,
        authorization_expires_at: p.authorization_expires_at,
    })
}

fn names_in(path: &Path, suffix: &str, names: &mut BTreeSet<String>) -> Result<()> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {
            return Err(Failure::new(
                "PROFILE_LIST_UNAVAILABLE",
                1,
                "无法列出本地账号配置。",
            ));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|_| invalid())?;
        if let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|v| v.strip_suffix(suffix))
            && !name.starts_with('.')
            && profile::validate_name(name).is_ok()
        {
            names.insert(name.into());
        }
    }
    Ok(())
}

fn names(rt: &Runtime) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    names_in(&rt.root, ".json", &mut names)?;
    names_in(&rt.root.join("governed"), ".profile.json", &mut names)?;
    Ok(names)
}

pub(crate) fn list(rt: &Runtime, details: bool) -> Result<Value> {
    let selected = private_store::read::<Selection>(&selection_path(&rt.root));
    let mut entries = Vec::new();
    for name in names(rt)? {
        match binding(rt, &name) {
            Ok(bound) => {
                let active = selected
                    .as_ref()
                    .ok()
                    .and_then(|v| v.as_ref())
                    .is_some_and(|s| s.schema_version == 1 && s.binding.same_identity(&bound));
                let mut item = serde_json::to_value(bound).map_err(|_| invalid())?;
                if !details {
                    for field in [
                        "subjectId",
                        "authorizationId",
                        "authorizationExpiresAt",
                        "issuer",
                    ] {
                        item.as_object_mut().unwrap().remove(field);
                    }
                }
                item["selected"] = json!(active);
                item["state"] = json!("local-record");
                if item["systemId"] == "supply-chain-server" {
                    item["deliveryCenterId"] =
                        json!(governed::selected_delivery_center(rt, &name)?);
                }
                entries.push(item);
            }
            Err(error) => entries.push(
                json!({"profile":name,"state":"invalid","selectable":false,"errorCode":error.code}),
            ),
        }
    }
    Ok(
        json!({"profiles":entries,"onlineVerified":false,"credentialReads":0,"httpRequests":0,"selectionState":if selected.is_err(){"invalid"}else{"local-record"}}),
    )
}

pub(crate) fn select(rt: &Runtime, name: &str) -> Result<Value> {
    let bound = binding(rt, name)?;
    private_store::write(
        &selection_path(&rt.root),
        &Selection {
            schema_version: 1,
            binding: bound.clone(),
        },
    )?;
    Ok(json!({"selected":bound,"onlineVerified":false,"credentialReads":0,"httpRequests":0}))
}

pub(crate) fn resolve(
    rt: &Runtime,
    explicit: Option<&str>,
    provider: &str,
    system: Option<&str>,
    write: bool,
) -> Result<Binding> {
    let mismatch = || {
        Failure::new(
            "PROFILE_SELECTION_MISMATCH",
            2,
            "所选账号与任务的系统或身份来源不匹配；请显式选择profile。",
        )
    };
    if let Some(name) = explicit {
        let bound = binding(rt, name)?;
        return if bound.matches(provider, system) {
            Ok(bound)
        } else {
            Err(mismatch())
        };
    }
    if write {
        return Err(Failure::new(
            "PROFILE_REQUIRED",
            2,
            "写任务必须显式指定profile。",
        ));
    }
    if let Some(selected) = private_store::read::<Selection>(&selection_path(&rt.root))? {
        if selected.schema_version != 1 {
            return Err(mismatch());
        }
        let current = binding(rt, &selected.binding.profile)?;
        if !selected.binding.same_identity(&current) || !current.matches(provider, system) {
            return Err(mismatch());
        }
        return Ok(current);
    }
    let candidates: Vec<_> = names(rt)?
        .iter()
        .filter_map(|n| binding(rt, n).ok())
        .filter(|b| b.matches(provider, system))
        .collect();
    match candidates.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(Failure::new(
            "PROFILE_NOT_CONFIGURED",
            2,
            "没有匹配的账号；请先执行setup。",
        )),
        _ => Err(Failure::new(
            "PROFILE_REQUIRED",
            2,
            "有多个匹配账号；请显式选择profile或执行profiles use。",
        )),
    }
}

pub(crate) async fn setup(
    rt: &Runtime,
    name: &str,
    environment: &str,
    system: Option<&str>,
) -> Result<Value> {
    profile::validate_name(name)?;
    rt.environment(environment)?;
    if profile::read(&rt.root, name)?.is_some() || governed::profile_exists(&rt.root, name)? {
        let current = binding(rt, name)?;
        if current.environment != environment || current.system_id.as_deref() != system {
            return Err(Failure::new(
                "PROFILE_SELECTION_MISMATCH",
                2,
                "既有账号绑定与setup目标不同；未修改原profile。",
            ));
        }
    }
    if let Some(system) = system {
        governed::setup(rt, name, environment, system).await
    } else {
        login::login(rt, name, Some(environment)).await
    }
}

#[cfg(test)]
mod tests;
