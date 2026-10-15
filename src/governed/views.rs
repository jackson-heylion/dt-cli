//! Employee view candidates are live metadata, never a profile-global current selection.
use super::*;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ViewOption {
    pub view_id: i32,
    pub view_type: String,
    pub data_type: i32,
    pub view_name: Option<String>,
    pub object_id: String,
    pub object_name: String,
    pub object_type: String,
    pub shop_code: Option<String>,
    pub brand_id: Option<i32>,
    pub parent_id: Option<i32>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
}

impl ViewOption {
    pub(super) fn valid(&self) -> bool {
        let data_type = match self.view_type.as_str() {
            "GROUP" => 3,
            "SHOP" => 4,
            "MANUFACTURE" => 6,
            "DELIVERY" => 7,
            "SUPPLIER" => 8,
            "BRAND" => 9,
            "FRANCHISE" => 2,
            _ => return false,
        };
        self.view_id > 0
            && self.data_type == data_type
            && !self.object_id.is_empty()
            && !self.object_name.is_empty()
            && (self.supported
                || self
                    .unsupported_reason
                    .as_ref()
                    .is_some_and(|reason| !reason.is_empty()))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Views {
    system_id: String,
    environment: String,
    subject_id: String,
    options: Vec<ViewOption>,
}

pub(super) async fn list(rt: &Runtime, name: &str) -> Result<Value> {
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    let options = candidates(rt, &p, env, &client).await?;
    Ok(
        json!({"systemId":p.system_id,"environment":p.environment,"subjectId":p.subject_id,
        "options":options,"onlineVerified":true,"permissionVerified":true}),
    )
}

pub(super) async fn candidates(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
) -> Result<Vec<ViewOption>> {
    let route = format!(
        "/cli-api/v1/systems/{}/{}",
        p.system_id,
        crate::constants::protocol::VIEWS_SUFFIX
    );
    let response = authorized(rt, p, env, client, |token| {
        Ok(client.get(endpoint(env, &route)?).bearer_auth(token))
    })
    .await?;
    let data = envelope(response)?["data"].clone();
    let views: Views = serde_json::from_value(data).map_err(|_| http::protocol())?;
    if views.system_id != p.system_id
        || views.environment != p.environment
        || views.subject_id != p.subject_id
    {
        return Err(identity_mismatch());
    }
    let mut seen = BTreeSet::new();
    for option in &views.options {
        if !option.valid() || !seen.insert((option.view_id, option.object_id.clone())) {
            return Err(http::protocol());
        }
    }
    Ok(views.options)
}

/// Infer only an absent context parameter after all supplied arguments have passed their schema.
pub(super) async fn infer(
    rt: &Runtime,
    name: &str,
    p: &GovernedProfile,
    operation: &Value,
    params: &mut Value,
) -> Result<()> {
    let identity = &operation["direct"]["execution"]["identity"];
    let Some(parameter) = identity["contextParameter"].as_str() else {
        return Ok(());
    };
    if params.get(parameter).is_some() {
        return Ok(());
    }
    let mut schema = operation["inputSchema"].clone();
    if let Some(required) = schema["required"].as_array_mut() {
        required.retain(|value| value != parameter);
    }
    let validator = jsonschema::draft202012::new(&schema).map_err(|_| catalog_invalid())?;
    if !validator.is_valid(params) {
        return Err(invalid());
    }
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    let expected = match identity["mode"].as_str() {
        Some("supply-chain.brand") => Some("BRAND"),
        Some("supply-chain.delivery") => Some("DELIVERY"),
        Some("supply-chain.view") => None,
        _ => {
            return Err(Failure::new(
                "VIEW_SELECTION_REQUIRED",
                2,
                "请根据接口说明明确选择任务视角。",
            ));
        }
    };
    let options: Vec<_> = candidates(rt, p, env, &client)
        .await?
        .into_iter()
        .filter(|option| expected.is_none_or(|kind| option.view_type == kind) && option.supported)
        .collect();
    if options.len() != 1 {
        return Err(Failure::new(
            "VIEW_SELECTION_REQUIRED",
            2,
            "任务视角不能唯一确定；请从本人可用对象中选择。",
        )
        .with_partial(
            json!({"profile":name,"availableViews":options}),
            json!({"complete":false}),
        ));
    }
    let selected = if expected.is_none() {
        json!({"viewId":options[0].view_id,"objectId":options[0].object_id})
    } else {
        json!(
            options[0]
                .object_id
                .parse::<i32>()
                .ok()
                .filter(|id| *id > 0)
                .ok_or_else(http::protocol)?
        )
    };
    params
        .as_object_mut()
        .ok_or_else(invalid)?
        .insert(parameter.to_owned(), selected);
    Ok(())
}
