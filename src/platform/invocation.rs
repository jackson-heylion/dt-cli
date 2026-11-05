//! 执行输入的有界解析、Schema 验证和 HTTP 序列化边界。
use super::{contract::*, input_model::*};
use crate::output::{Failure, Result, invalid};
use serde_json::Value;
use std::io::Read;

pub(super) fn input_file(path: &str, body_file: Option<&str>) -> Result<InvocationInput> {
    fn bounded(path: &str, limit: u64) -> Result<String> {
        let file = std::fs::File::open(path).map_err(|_| invalid())?;
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() as u64 > limit {
            return Err(invalid());
        }
        String::from_utf8(bytes).map_err(|_| invalid())
    }
    let mut input: InvocationInput =
        serde_json::from_str(&bounded(path, 131072)?).map_err(|_| invalid())?;
    if let Some(file) = body_file {
        if input.body_json.is_some() {
            return Err(invalid());
        }
        input.body_json = Some(bounded(file, 65536)?);
    }
    Ok(input)
}
fn scalar(value: &Scalar) -> Result<String> {
    match value {
        Scalar::Text(v) => Ok(v.clone()),
        Scalar::Boolean(v) => Ok(v.to_string()),
        Scalar::Number(v) => Ok(v.to_string()),
        Scalar::Null => Err(Failure::new(
            "URL_NULL_UNSUPPORTED",
            2,
            "URL、业务头和普通表单不传输 null；请省略可选参数或使用 JSON 请求体。",
        )),
    }
}
fn json_value(value: &ArgumentValue) -> Result<Value> {
    match value {
        ArgumentValue::Scalar(v) => serde_json::to_value(v).map_err(|_| invalid()),
        ArgumentValue::Array(v) => serde_json::to_value(v).map_err(|_| invalid()),
        ArgumentValue::Object(v) => {
            let mut object = serde_json::Map::new();
            for property in &v.properties {
                if object
                    .insert(
                        property.name.clone(),
                        serde_json::to_value(&property.value).map_err(|_| invalid())?,
                    )
                    .is_some()
                {
                    return Err(invalid());
                }
            }
            Ok(Value::Object(object))
        }
    }
}
pub(super) fn validate(schema: &str, document: &str) -> Result<()> {
    let schema: Value = serde_json::from_str(schema).map_err(|_| unsupported())?;
    if !resolved(&schema, 0) {
        return Err(unsupported());
    }
    let validator = jsonschema::draft202012::new(&schema).map_err(|_| unsupported())?;
    let value: Value = serde_json::from_str(document).map_err(|_| invalid())?;
    if !validator.is_valid(&value) {
        return Err(Failure::new(
            "PARAMETER_SCHEMA_MISMATCH",
            2,
            "输入不符合固定执行版本的 Schema，未发出业务请求。",
        ));
    }
    Ok(())
}
fn resolved(value: &Value, depth: u16) -> bool {
    if depth > 64 {
        return false;
    }
    match value {
        Value::Object(object) => {
            !object.get("$ref").is_some_and(Value::is_string)
                && !object.contains_key("$dynamicRef")
                && !object.contains_key("$recursiveRef")
                && object.values().all(|v| resolved(v, depth + 1))
        }
        Value::Array(array) => array.iter().all(|v| resolved(v, depth + 1)),
        _ => true,
    }
}
fn unsupported() -> Failure {
    Failure::new(
        "EXECUTION_UNSUPPORTED",
        2,
        "固定契约的 Schema 或参数序列化不受此客户端支持，请先处理平台校验原因。",
    )
}
fn reserved(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "host",
        "connection",
        "content-length",
        "content-type",
        "transfer-encoding",
        "upgrade",
        "te",
        "trailer",
        "forwarded",
    ]
    .contains(&n.as_str())
        || [
            "x-platform-",
            "x-cliapi-",
            "x-forwarded-",
            "x-real-",
            "sec-",
        ]
        .iter()
        .any(|p| n.starts_with(p))
}
fn pairs(parameter: &Parameter, value: &ArgumentValue) -> Result<Vec<(String, String)>> {
    let name = &parameter.name;
    let simple = parameter.location == Location::Path || parameter.location == Location::Header;
    if parameter
        .style
        .as_deref()
        .unwrap_or(if simple { "simple" } else { "form" })
        != if simple { "simple" } else { "form" }
    {
        return Err(unsupported());
    }
    let pieces = match value {
        ArgumentValue::Scalar(v) => return Ok(vec![(name.clone(), scalar(v)?)]),
        ArgumentValue::Array(values) => {
            let values = values.iter().map(scalar).collect::<Result<Vec<_>>>()?;
            if values.iter().any(|v| v.contains([',', '='])) {
                return Err(unsupported());
            }
            if !simple && parameter.explode {
                return Ok(values
                    .into_iter()
                    .map(|value| (name.clone(), value))
                    .collect());
            }
            values
        }
        ArgumentValue::Object(object) => {
            let mut pieces = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for item in &object.properties {
                let value = scalar(&item.value)?;
                if !seen.insert(&item.name)
                    || item.name.contains([',', '='])
                    || value.contains([',', '='])
                {
                    return Err(unsupported());
                }
                if !simple && parameter.explode {
                    pieces.push((item.name.clone(), value));
                } else if parameter.explode {
                    pieces.push((String::new(), format!("{}={value}", item.name)));
                } else {
                    pieces.push((String::new(), item.name.clone()));
                    pieces.push((String::new(), value));
                }
            }
            if !simple && parameter.explode {
                return Ok(pieces);
            }
            pieces.into_iter().map(|(_, value)| value).collect()
        }
    };
    Ok(vec![(name.clone(), pieces.join(","))])
}
pub(super) fn encode(
    contract: &ExecutionContract,
    input: &InvocationInput,
) -> Result<WireInvocation> {
    let mut path = contract.path.clone();
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    let mut headers = Vec::new();
    for (location, arguments) in [
        (Location::Path, &input.path),
        (Location::Query, &input.query),
        (Location::Header, &input.headers),
        (Location::Form, &input.form),
    ] {
        let mut seen = std::collections::HashSet::new();
        for argument in arguments {
            if !seen.insert(argument.name.as_str()) {
                return Err(invalid());
            }
            let p = contract
                .parameters
                .iter()
                .find(|p| p.location == location && p.name == argument.name)
                .ok_or_else(invalid)?;
            let raw = json_value(&argument.value)?;
            validate(
                p.schema.as_deref().ok_or_else(unsupported)?,
                &raw.to_string(),
            )?;
            let serialized = pairs(p, &argument.value)?;
            for (name, value) in serialized {
                match location {
                    Location::Path => {
                        if value.is_empty()
                            || value.contains(['/', '%', '\\', ';', '?', '#'])
                            || value == "."
                            || value == ".."
                            || !value.is_ascii()
                            || value.bytes().any(|b| b < 33)
                        {
                            return Err(invalid());
                        }
                        path = path.replace(&format!("{{{name}}}"), &value);
                    }
                    Location::Query => {
                        query.append_pair(&name, &value);
                    }
                    Location::Form => {
                        form.append_pair(&name, &value);
                    }
                    Location::Header => {
                        if reserved(&name) || value.contains(['\r', '\n']) {
                            return Err(invalid());
                        }
                        headers.push(BusinessHeader { name, value });
                    }
                }
            }
        }
        for p in contract
            .parameters
            .iter()
            .filter(|p| p.location == location && p.required)
        {
            if !seen.contains(p.name.as_str()) {
                return Err(Failure::new(
                    "REQUIRED_PARAMETER",
                    2,
                    "缺少固定契约要求的必填参数。",
                ));
            }
        }
    }
    if path.contains(['{', '}']) {
        return Err(invalid());
    }
    let body = match contract.media {
        Media::None => {
            if input.body_json.is_some() || !input.form.is_empty() {
                return Err(invalid());
            }
            String::new()
        }
        Media::Json => {
            if !input.form.is_empty() {
                return Err(invalid());
            }
            match &input.body_json {
                Some(raw) => {
                    validate(
                        contract.request_schema.as_deref().ok_or_else(unsupported)?,
                        raw,
                    )?;
                    raw.clone()
                }
                None if !contract.request_body_required => String::new(),
                None => return Err(invalid()),
            }
        }
        Media::Form => {
            if input.body_json.is_some() {
                return Err(invalid());
            }
            if let Some(schema) = &contract.request_schema {
                let mut object = serde_json::Map::new();
                for argument in &input.form {
                    object.insert(argument.name.clone(), json_value(&argument.value)?);
                }
                validate(schema, &Value::Object(object).to_string())?;
            }
            if contract.request_body_required && input.form.is_empty() {
                return Err(invalid());
            }
            form.finish()
        }
    };
    let query = query.finish();
    if body.len() > 65536 || query.len() > 8192 || headers.len() > 100 {
        return Err(invalid());
    }
    Ok(WireInvocation {
        path,
        query,
        headers,
        body,
        view: input.view.clone(),
    })
}
