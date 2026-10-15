//! Typed skill dependencies and the permissions saved when a business run starts.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SkillRequirements {
    pub schema_version: u32,
    pub contract_schema_version: u64,
    pub system_id: String,
    pub operations: Vec<RequiredOperation>,
    pub python_minimum_version: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RequiredOperation {
    pub operation_id: String,
    pub operation_version: String,
    pub effect: String,
    #[serde(default = "required")]
    pub required: bool,
}
fn required() -> bool {
    true
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SkillContract {
    pub operation_id: String,
    pub operation_version: String,
    pub contract_digest: String,
    pub effect: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnavailableOperation {
    pub operation_id: String,
    pub code: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SkillPermission {
    pub subject_id: String,
    pub environment: String,
    pub system_id: String,
    pub authorization_id: String,
    pub contracts: Vec<SkillContract>,
    pub unavailable: Vec<UnavailableOperation>,
}
/// Current authority is checked before the task records exact versions. No credential enters the record.
pub(crate) async fn prepare_skill(
    rt: &Runtime,
    name: &str,
    requirements: &SkillRequirements,
) -> Result<SkillPermission> {
    let p = require_profile(rt, name)?;
    if requirements.schema_version != crate::constants::protocol::FILE_SCHEMA
        || requirements.contract_schema_version != CONTRACT_SCHEMA_VERSION
        || requirements.system_id != p.system_id
        || requirements.operations.len() > crate::constants::query::DISCOVERY_LIMIT
        || !requirements.dependencies.is_empty()
        || requirements
            .python_minimum_version
            .as_deref()
            .is_some_and(|v| v != crate::constants::clients::SUPPORTED_PYTHON_MINIMUM)
    {
        return Err(catalog_invalid());
    }
    catalog::sync(rt, name).await?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let mut contracts = Vec::new();
    let mut unavailable = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for entry in &requirements.operations {
        let id = &entry.operation_id;
        let version = &entry.operation_version;
        if !valid_id(id)
            || catalog::version_triple(version).is_none()
            || entry.effect != "read"
            || !seen.insert(id)
        {
            return Err(catalog_invalid());
        }
        match catalog::select(&cache, id, Some(version)).and_then(|op| {
            usable(op, "read")?;
            Ok(op)
        }) {
            Ok(op) => contracts.push(SkillContract {
                operation_id: id.clone(),
                operation_version: version.clone(),
                contract_digest: op["contractDigest"]
                    .as_str()
                    .ok_or_else(catalog_invalid)?
                    .to_owned(),
                effect: "read".to_owned(),
            }),
            Err(error) => {
                if entry.required {
                    return Err(error);
                }
                unavailable.push(UnavailableOperation {
                    operation_id: id.clone(),
                    code: error.code.to_owned(),
                });
            }
        }
    }
    Ok(SkillPermission {
        subject_id: p.subject_id,
        environment: p.environment,
        system_id: p.system_id,
        authorization_id: p.authorization_id,
        contracts,
        unavailable,
    })
}
