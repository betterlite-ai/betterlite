#![cfg(test)]

use super::*;
#[test]
fn error_uses_error_string_fallback() {
    let payload =
        serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"status":"failure","error_str":"failed","error_information":{"error_class":"ProviderError","error_code":"E1","traceback":"trace"}}"#)
            .unwrap();
    let error = SpanError::from_payload(&payload).unwrap();
    assert_eq!(error.message.as_deref(), Some("failed"));
    assert_eq!(error.error_type.as_deref(), Some("ProviderError"));
    assert_eq!(error.code.as_deref(), Some("E1"));
    assert_eq!(error.stack_trace.as_deref(), Some("trace"));
}

#[test]
fn error_is_absent_for_non_failure_status() {
    let payload = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(
        r#"{"status":"success","error_information":{"error_class":"ProviderError"}}"#,
    )
    .unwrap();
    assert!(SpanError::from_payload(&payload).is_none());
}

#[test]
fn error_class_falls_back_to_error_code() {
    let payload = serde_json::from_str::<BTreeMap<String, OtelModelValue>>(r#"{"status":"failure","error_information":{"error_code":"E42","error_message":"bad response"}}"#).unwrap();
    let error = SpanError::from_payload(&payload).unwrap();
    assert_eq!(error.error_type.as_deref(), Some("E42"));
    assert_eq!(error.code.as_deref(), Some("E42"));
    assert_eq!(error.message.as_deref(), Some("bad response"));
}
