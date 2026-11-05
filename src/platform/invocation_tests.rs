use super::{contract::*, input_model::*, invocation};

fn contract(media: Media, parameters: Vec<Parameter>) -> ExecutionContract {
    ExecutionContract {
        contract_schema_version: 1, operation_id: "orders".into(), operation_version: 1, minimum_cli_version: "0.6.0".into(),
        system_id: "orders".into(), environment: "fixture".into(), title: "查询".into(), method: Method::Post, path: "/orders/{id}".into(),
        effect: Effect::Read, media, parameters, request_schema: Some("{\"type\":\"object\",\"required\":[\"note\"],\"properties\":{\"note\":{\"type\":[\"string\",\"null\"]}},\"additionalProperties\":false}".into()),
        response_schema: Some("{}".into()), success: Success { status_codes: vec![200], body_pointer: None, expected_json: None },
        pagination: Pagination { kind: PaginationKind::None, page_parameter: None, size_parameter: None, items_pointer: None, next_pointer: None, total_pointer: None, first_page: 1, page_size: 100, placement: PaginationPlacement::Query },
        view_types: vec![], data_scope: "self".into(), sensitive_fields: vec![], route_binding_digest: "route".into(),
        limits: Limits { max_pages: 100, max_records: 10000, max_bytes: 2097152, max_seconds: 120 }, request_body_required: true,
    }
}
fn parameter(name: &str, location: Location, schema: &str) -> Parameter {
    let style = if location == Location::Path || location == Location::Header {
        "simple"
    } else {
        "form"
    };
    Parameter {
        name: name.into(),
        location,
        required: true,
        schema: Some(schema.into()),
        style: Some(style.into()),
        explode: true,
    }
}
#[test]
fn encodes_explicit_path_query_header_and_keeps_json_bytes() {
    let c = contract(
        Media::Json,
        vec![
            parameter("id", Location::Path, "{\"type\":\"integer\"}"),
            parameter(
                "tags",
                Location::Query,
                "{\"type\":\"array\",\"items\":{\"type\":\"string\"}}",
            ),
            parameter("Language", Location::Header, "{\"type\":\"string\"}"),
        ],
    );
    let input:InvocationInput=serde_json::from_str(r#"{"path":[{"name":"id","value":12}],"query":[{"name":"tags","value":["a b","中"]}],"headers":[{"name":"Language","value":"zh-CN"}],"bodyJson":"{ \"note\": null }"}"#).unwrap();
    let wire = invocation::encode(&c, &input).unwrap();
    assert_eq!(wire.path, "/orders/12");
    assert_eq!(wire.query, "tags=a+b&tags=%E4%B8%AD");
    assert_eq!(wire.body, "{ \"note\": null }");
    assert_eq!(wire.headers[0].name, "Language");
    assert_eq!(wire.headers[0].value, "zh-CN");
}
#[test]
fn refuses_invalid_body_missing_arguments_and_managed_headers() {
    let c = contract(
        Media::Json,
        vec![parameter("id", Location::Path, "{\"type\":\"integer\"}")],
    );
    let input: InvocationInput = serde_json::from_str(
        r#"{"path":[{"name":"id","value":12}],"bodyJson":"{\"extra\":true}"}"#,
    )
    .unwrap();
    assert!(invocation::encode(&c, &input).is_err());
    let mut missing = input.clone();
    missing.path.clear();
    assert!(invocation::encode(&c, &missing).is_err());
    let mut spoof = input;
    spoof.body_json = Some("{\"note\":null}".into());
    spoof.headers = vec![Argument {
        name: "Authorization".into(),
        value: ArgumentValue::Scalar(Scalar::Text("synthetic-private-value".into())),
    }];
    let error = invocation::encode(&c, &spoof).err().unwrap();
    assert!(
        !serde_json::to_string(&error)
            .unwrap_or_default()
            .contains("synthetic-private-value")
    );
}
#[test]
fn ordinary_form_is_encoded_and_checked_as_the_declared_shape() {
    let mut c = contract(
        Media::Form,
        vec![
            parameter("id", Location::Path, "{\"type\":\"integer\"}"),
            parameter(
                "tags",
                Location::Form,
                "{\"type\":\"array\",\"items\":{\"type\":\"string\"}}",
            ),
            parameter("enabled", Location::Form, "{\"type\":\"boolean\"}"),
        ],
    );
    c.request_schema=Some("{\"type\":\"object\",\"required\":[\"tags\",\"enabled\"],\"additionalProperties\":false,\"properties\":{\"tags\":{\"type\":\"array\",\"items\":{\"type\":\"string\"}},\"enabled\":{\"type\":\"boolean\"}}}".into());
    let input:InvocationInput=serde_json::from_str(r#"{"path":[{"name":"id","value":1}],"form":[{"name":"tags","value":["a","b"]},{"name":"enabled","value":true}]}"#).unwrap();
    assert_eq!(
        invocation::encode(&c, &input).unwrap().body,
        "tags=a&tags=b&enabled=true"
    );
}

#[test]
fn validates_decimal_bounds_without_rounding_to_binary_float() {
    let schema = r#"{"type":"number","exclusiveMinimum":123.0000000000000000000000001}"#;
    assert!(super::invocation::validate(schema, "123.0000000000000000000000002").is_ok());
    assert!(super::invocation::validate(schema, "123.0000000000000000000000001").is_err());
    assert!(super::invocation::validate(schema, "123.0000000000000000000000000").is_err());
}
