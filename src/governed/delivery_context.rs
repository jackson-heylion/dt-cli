use super::*;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeliveryContext {
    schema_version: u32,
    subject_id: String,
    issuer: String,
    environment: String,
    system_id: String,
    delivery_center_id: i32,
}
fn path(rt: &Runtime, name: &str) -> Result<PathBuf> {
    profile::validate_name(name)?;
    Ok(directory(&rt.root)
        .join("query-context")
        .join(format!("{name}.delivery.json")))
}
pub(crate) fn set_delivery_center(rt: &Runtime, name: &str, id: i32) -> Result<Value> {
    let p = require_profile(rt, name)?;
    if p.system_id != "supply-chain-server" || id <= 0 {
        return Err(invalid());
    }
    crate::private_store::write(
        &path(rt, name)?,
        &DeliveryContext {
            schema_version: 1,
            subject_id: p.subject_id,
            issuer: p.issuer,
            environment: p.environment,
            system_id: p.system_id,
            delivery_center_id: id,
        },
    )?;
    Ok(
        json!({"profile":name,"deliveryCenterId":id,"onlineVerified":false,"permissionVerified":false,"httpRequests":0,"credentialReads":0}),
    )
}
// Defaults apply only when the exact chosen contract declares this field; explicit input wins.
pub(super) fn apply(
    rt: &Runtime,
    name: &str,
    p: &GovernedProfile,
    operation: &Value,
    params: &mut Value,
) -> Result<()> {
    if p.system_id != "supply-chain-server"
        || params.get("deliveryCenterId").is_some()
        || operation["inputSchema"]["properties"]
            .get("deliveryCenterId")
            .is_none()
        || operation["operationId"] == "supply-chain-server.delivery-centers.list"
    {
        return Ok(());
    }
    let Some(c) = crate::private_store::read::<DeliveryContext>(&path(rt, name)?)? else {
        return Ok(());
    };
    if c.schema_version != 1
        || c.delivery_center_id <= 0
        || c.subject_id != p.subject_id
        || c.issuer != p.issuer
        || c.environment != p.environment
        || c.system_id != p.system_id
    {
        return Err(profile_invalid());
    }
    params
        .as_object_mut()
        .ok_or_else(invalid)?
        .insert("deliveryCenterId".into(), json!(c.delivery_center_id));
    Ok(())
}

pub(crate) fn selected(rt: &Runtime, name: &str) -> Result<Option<i32>> {
    let Some(c) = crate::private_store::read::<DeliveryContext>(&path(rt, name)?)? else {
        return Ok(None);
    };
    let p = require_profile(rt, name)?;
    if c.schema_version != 1
        || c.delivery_center_id <= 0
        || c.subject_id != p.subject_id
        || c.issuer != p.issuer
        || c.environment != p.environment
        || c.system_id != p.system_id
    {
        return Err(profile_invalid());
    }
    Ok(Some(c.delivery_center_id))
}
