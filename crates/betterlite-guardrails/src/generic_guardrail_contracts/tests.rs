#![cfg(test)]

use super::headers::{InboundRequestContext, extract_inbound_headers, sanitize_inbound_headers};
use super::*;
use serde_json::from_str;
use std::collections::BTreeMap;

#[test]
fn generic_guardrail_header_allowlist_and_globs() {
    let headers = BTreeMap::from([
        (
            "Host".to_owned(),
            GuardrailExtraValue::Text("guardrail.example".to_owned()),
        ),
        (
            "X-Stainless-Raw".to_owned(),
            GuardrailExtraValue::Text("safe-value".to_owned()),
        ),
        (
            "Authorization".to_owned(),
            GuardrailExtraValue::Text("secret".to_owned()),
        ),
    ]);
    let sanitized = sanitize_inbound_headers(&headers, None);
    assert_eq!(sanitized.get("Host"), Some(&"guardrail.example".to_owned()));
    assert_eq!(
        sanitized.get("X-Stainless-Raw"),
        Some(&"safe-value".to_owned())
    );
    assert_eq!(
        sanitized.get("Authorization"),
        Some(&"[present]".to_owned())
    );
}

#[test]
fn generic_guardrail_header_extraction_uses_proxy_first() {
    let request_data: InboundRequestContext = from_str(
        r#"{"proxy_server_request":{"headers":{"Authorization":"secret"}},"metadata":{"headers":{"Host":"wrong.example"}}}"#,
    )
    .expect("request context");
    let headers = extract_inbound_headers(&request_data, None).expect("headers");
    assert_eq!(headers.get("Authorization"), Some(&"[present]".to_owned()));
    assert!(!headers.contains_key("Host"));
}

#[test]
fn generic_guardrail_header_extraction_uses_logging_fallback() {
    let request_data: InboundRequestContext = from_str(
        r#"{"logging_obj":{"model_call_details":{"litellm_params":{"metadata":{"headers":{"X-Request-Id":"request-123"}}}}}}"#,
    )
    .expect("request context");
    let headers = extract_inbound_headers(&request_data, None).expect("headers");
    assert_eq!(headers.get("X-Request-Id"), Some(&"[present]".to_owned()));
}

#[test]
fn generic_guardrail_header_extraction_uses_nested_extra_allowlist() {
    let request_data: InboundRequestContext = from_str(
        r#"{"litellm_params":{"extra_headers":["X-Request-Id"],"metadata":{"headers":{"X-Request-Id":"request-123"}}}}"#,
    )
    .expect("request context");
    let headers = extract_inbound_headers(&request_data, None).expect("headers");
    assert_eq!(headers.get("X-Request-Id"), Some(&"request-123".to_owned()));
}

#[test]
fn generic_guardrail_header_extraction_coerces_json_values() {
    let request_data: InboundRequestContext =
        from_str(
            r#"{"metadata":{"headers":{"Host":false,"X-Stainless-Raw":[1,2,3],"X-LiteLLM-Meta":{"region":"eu"},"X-Request-Id":"request-123"}}}"#,
        )
        .expect("request context");
    let headers = extract_inbound_headers(&request_data, None).expect("headers");
    assert_eq!(headers.get("Host"), Some(&"False".to_owned()));
    assert_eq!(
        headers.get("X-Stainless-Raw"),
        Some(&"[1, 2, 3]".to_owned())
    );
    assert_eq!(
        headers.get("X-LiteLLM-Meta"),
        Some(&"{'region': 'eu'}".to_owned())
    );
    assert_eq!(headers.get("X-Request-Id"), Some(&"[present]".to_owned()));
}

#[test]
fn generic_guardrail_constructor_defaults() {
    let settings = GenericGuardrailAPISettings::from_optional(None);
    assert_eq!(
        settings.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    assert!(settings.fail_on_error);
    assert_eq!(settings.streaming_sampling_rate, 5);
    assert_eq!(
        settings.streaming_transform_mode,
        StreamingTransformMode::BlockOnly
    );
}

#[test]
fn generic_guardrail_contracts_reject_unknown_closed_values() {
    drop(from_str::<GuardrailOptions>(r#"{"unreachable_fallback":"typo"}"#).unwrap_err());
    drop(from_str::<GuardrailOptions>(r#"{"streaming_transform_mode":"typo"}"#).unwrap_err());
}

#[test]
fn generic_guardrail_response_coerces_holdback_values() {
    let response: GenericGuardrailAPIResponse =
        from_str(r#"{"stream_holdback_chars":[null,-2,"3",false,"bad"]}"#).expect("response");
    assert_eq!(response.stream_holdback_chars, Some(vec![0, 0, 3, 0, 0]));
}

#[test]
fn generic_guardrail_response_defaults_and_types_action() {
    let default_response: GenericGuardrailAPIResponse = from_str("{}").expect("response");
    assert_eq!(default_response.action, GuardrailResponseAction::None);
    let intervened_response: GenericGuardrailAPIResponse =
        from_str(r#"{"action":"GUARDRAIL_INTERVENED"}"#).expect("response");
    assert_eq!(
        intervened_response.action,
        GuardrailResponseAction::GuardrailIntervened
    );
    drop(from_str::<GenericGuardrailAPIResponse>(r#"{"action":"UNKNOWN"}"#).unwrap_err());
}

#[test]
fn generic_guardrail_row_reconciliation() {
    let original: Vec<GuardrailMessage> =
        from_str(r#"[{"role":"user","id":"original"}]"#).expect("original");
    let shown: Vec<GuardrailMessage> =
        from_str(r#"[{"role":"user","content":"shown"}]"#).expect("shown");
    let echoed: Vec<GuardrailMessage> =
        from_str(r#"[{"role":"user","content":"shown"}]"#).expect("echoed");
    assert_eq!(reconcile_structured_rows(&original, &shown, &echoed), None);
    let changed: Vec<GuardrailMessage> =
        from_str(r#"[{"role":"user","content":"redacted"}]"#).expect("changed");
    let reconciled = reconcile_structured_rows(&original, &shown, &changed).expect("rows");
    assert_eq!(reconciled, changed);
}

#[test]
fn generic_guardrail_tool_param_preserves_extra_fields() {
    let tool: GuardrailToolParam =
        from_str(r#"{"type":"code_interpreter","container":{"type":"auto"}}"#).expect("tool");
    assert!(tool.extra.contains_key("container"));
}
