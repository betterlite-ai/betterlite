#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::body::ProxyErrorBody;
use super::model_access_denied::ModelAccessDeniedProxyException;
use super::{ExceptionCode, ProxyException, ProxyExceptionInit};

fn exception_with(message: &str, code: ExceptionCode) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: message.to_owned(),
        error_type: "bad_request_error".to_owned(),
        param: None,
        code,
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

#[rstest]
#[case::absent(ExceptionCode::Absent, "None")]
#[case::status(ExceptionCode::Status(429), "429")]
#[case::text(ExceptionCode::Text("tag_limited".to_owned()), "tag_limited")]
fn code_is_stored_as_text(#[case] code: ExceptionCode, #[case] expected: &str) {
    assert_eq!(exception_with("boom", code).code, expected);
}

#[rstest]
#[case::absent_stays_absent(ExceptionCode::Absent, None, None)]
#[case::status_falls_back(ExceptionCode::Status(400), None, Some("400"))]
#[case::text_falls_back(ExceptionCode::Text("tag_limited".to_owned()), None, Some("tag_limited"))]
#[case::empty_falls_back(ExceptionCode::Status(400), Some(String::new()), Some("400"))]
#[case::given_is_kept(
    ExceptionCode::Status(400),
    Some("openai_400".to_owned()),
    Some("openai_400")
)]
fn openai_code_falls_back_to_the_given_code(
    #[case] code: ExceptionCode,
    #[case] openai_code: Option<String>,
    #[case] expected: Option<&str>,
) {
    let init = ProxyExceptionInit {
        message: "boom".to_owned(),
        error_type: "bad_request_error".to_owned(),
        param: None,
        code,
        headers: BTreeMap::new(),
        openai_code,
        provider_specific_fields: BTreeMap::new(),
    };
    let exception = ProxyException::new(init);
    assert_eq!(exception.openai_code.as_deref(), expected);
}

#[rstest]
#[case::no_healthy_deployment("No healthy deployment available for this model", "429")]
#[case::no_deployments("No deployments available for selected model", "429")]
fn no_deployment_messages_force_429(#[case] message: &str, #[case] expected: &str) {
    assert_eq!(
        exception_with(message, ExceptionCode::Status(500)).code,
        expected
    );
}

#[test]
fn tag_routing_message_forces_401() {
    assert_eq!(
        exception_with(
            "Not allowed to access model due to tags configuration",
            ExceptionCode::Status(403)
        )
        .code,
        "401"
    );
}

#[test]
fn no_deployment_rule_wins_over_the_tag_routing_rule() {
    assert_eq!(
        exception_with(
            "No healthy deployment available; Not allowed to access model due to tags configuration",
            ExceptionCode::Status(500)
        )
        .code,
        "429"
    );
}

#[test]
fn openai_code_keeps_the_given_code_when_a_message_rule_rewrites_the_code() {
    let exception = exception_with("No deployments available", ExceptionCode::Status(500));
    assert_eq!(exception.code, "429");
    assert_eq!(exception.openai_code.as_deref(), Some("500"));
}

#[test]
fn headers_and_provider_fields_pass_through() {
    let mut headers = BTreeMap::new();
    headers.insert("x-request-id".to_owned(), "abc".to_owned());
    let mut provider_fields = BTreeMap::new();
    provider_fields.insert("provider".to_owned(), serde_json::json!("azure"));
    let exception = ProxyException::new(ProxyExceptionInit {
        message: "boom".to_owned(),
        error_type: "bad_request_error".to_owned(),
        param: Some("model".to_owned()),
        code: ExceptionCode::Status(400),
        headers,
        openai_code: None,
        provider_specific_fields: provider_fields,
    });
    assert_eq!(
        exception.headers.get("x-request-id").map(String::as_str),
        Some("abc")
    );
    assert_eq!(
        exception.provider_specific_fields.get("provider"),
        Some(&serde_json::json!("azure"))
    );
}

#[test]
fn body_lists_message_type_param_and_code_in_that_order() {
    let exception = exception_with("boom", ExceptionCode::Status(400));
    assert_eq!(
        serde_json::to_string(&exception.to_body()).unwrap(),
        r#"{"message":"boom","type":"bad_request_error","param":null,"code":"400"}"#
    );
}

#[test]
fn body_carries_provider_specific_fields_when_the_map_has_entries() {
    let mut provider_specific_fields = BTreeMap::new();
    provider_specific_fields.insert("provider".to_owned(), serde_json::json!("azure"));
    let exception = ProxyException::new(ProxyExceptionInit {
        message: "boom".to_owned(),
        error_type: "bad_request_error".to_owned(),
        param: None,
        code: ExceptionCode::Status(400),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields,
    });
    assert_eq!(
        serde_json::to_string(&exception.to_body()).unwrap(),
        r#"{"message":"boom","type":"bad_request_error","param":null,"code":"400","provider_specific_fields":{"provider":"azure"}}"#
    );
}

#[test]
fn body_leaves_out_an_empty_provider_specific_fields_map() {
    let body = ProxyErrorBody {
        message: "boom".to_owned(),
        error_type: "bad_request_error".to_owned(),
        param: Some("model".to_owned()),
        code: "400".to_owned(),
        provider_specific_fields: BTreeMap::new(),
    };
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"message":"boom","type":"bad_request_error","param":"model","code":"400"}"#
    );
}

#[test]
fn display_prints_the_message() {
    assert_eq!(
        exception_with("boom", ExceptionCode::Status(400)).to_string(),
        "boom"
    );
}

#[test]
fn model_access_denied_keeps_the_client_message_and_strips_line_breaks() {
    let denied = ModelAccessDeniedProxyException::new(
        "model access denied".to_owned(),
        "line one\r\nline two\nline three".to_owned(),
        "key_model_access_denied".to_owned(),
        None,
        ExceptionCode::Status(403),
    );
    assert_eq!(
        denied.sanitized_internal_message(),
        "line oneline twoline three"
    );
    assert_eq!(denied.as_proxy_exception().message, "model access denied");
    assert_eq!(denied.as_proxy_exception().code, "403");
    assert_eq!(
        denied.as_proxy_exception().openai_code.as_deref(),
        Some("403")
    );
}

#[test]
fn model_access_denied_carries_no_headers_or_provider_fields() {
    let denied = ModelAccessDeniedProxyException::new(
        "model access denied".to_owned(),
        "internal".to_owned(),
        "key_model_access_denied".to_owned(),
        Some("model".to_owned()),
        ExceptionCode::Status(403),
    );
    let exception = denied.as_proxy_exception();
    assert!(exception.headers.is_empty());
    assert!(exception.provider_specific_fields.is_empty());
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(
        serde_json::to_string(&exception.to_body()).unwrap(),
        r#"{"message":"model access denied","type":"key_model_access_denied","param":"model","code":"403"}"#
    );
}
