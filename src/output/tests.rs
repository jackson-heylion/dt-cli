//! 错误载荷堆存储保持协议不变，同时约束跨层返回的栈占用。
use super::*;

#[test]
fn boxed_diagnostics_keep_the_same_json_shape_and_bounded_error_size() {
    // 原内联诊断错误为864字节；堆分配后不超过现有恢复字段所需的288字节。
    assert!(std::mem::size_of::<Failure>() <= 288);
    let mut failure = Failure::new("NETWORK_ERROR", 5, "网络请求未完成。");
    failure.details = Some(Box::new(Details {
        login_stage: Some("exchanging_code".into()),
        http_status: Some(502),
        reason: Some("proxy_connect_failed".into()),
        ..Default::default()
    }));
    let (value, code) = envelope_result("login", None, Err(failure));
    assert_eq!(code, 5);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["details"]["loginStage"], "exchanging_code");
    assert_eq!(value["error"]["details"]["httpStatus"], 502);
    assert_eq!(value["error"]["details"]["reason"], "proxy_connect_failed");
    assert!(value["error"]["details"].get("processId").is_none());
}
