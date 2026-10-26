#![cfg(test)]

use super::*;
use serde_json::from_str;
use std::collections::BTreeMap;

#[test]
fn generic_guardrail_header_allowlist_and_globs() {
    let headers = BTreeMap::from([
        ("Host".to_owned(), "guardrail.example".to_owned()),
        ("X-Stainless-Raw".to_owned(), "safe-value".to_owned()),
        ("Authorization".to_owned(), "secret".to_owned()),
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
fn generic_guardrail_constructor_defaults() {
    let settings = GenericGuardrailAPISettings::from_optional(None);
    assert_eq!(settings.unreachable_fallback, "fail_closed");
    assert!(settings.fail_on_error);
    assert_eq!(settings.streaming_sampling_rate, 5);
    assert_eq!(settings.streaming_transform_mode, "block_only");
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
