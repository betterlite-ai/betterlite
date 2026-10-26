#![cfg(test)]

use super::*;
use serde_json::json;
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
    let request_data = json!({
        "proxy_server_request": {"headers": {"Authorization": "secret"}},
        "metadata": {"headers": {"Host": "wrong.example"}}
    });
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
    let original = vec![json!({"role":"user", "id":"original"})];
    let shown = vec![json!({"role":"user", "content":"shown"})];
    let echoed = reconcile_structured_rows(
        &original,
        &shown,
        &[json!({"role":"user", "content":"shown"})],
    );
    assert_eq!(echoed, None);
    let changed = reconcile_structured_rows(
        &original,
        &shown,
        &[json!({"role":"user", "content":"redacted"})],
    );
    assert_eq!(
        changed,
        Some(vec![json!({"role":"user", "content":"redacted"})])
    );
}

#[test]
fn generic_guardrail_tool_param_preserves_extra_fields() {
    let tool: GuardrailToolParam = serde_json::from_value(json!({
        "type": "code_interpreter",
        "container": {"type": "auto"}
    }))
    .expect("tool");
    assert_eq!(tool.extra.get("container"), Some(&json!({"type": "auto"})));
}

#[test]
fn generic_guardrail_response_coerces_holdback_values() {
    let response = GenericGuardrailAPIResponse::from_value(json!({
        "stream_holdback_chars": [null, -2, "3", "bad"]
    }))
    .expect("response");
    assert_eq!(response.stream_holdback_chars, Some(vec![0, 0, 3, 0]));
}
