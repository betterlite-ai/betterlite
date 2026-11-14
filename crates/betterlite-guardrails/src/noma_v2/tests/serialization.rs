use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::noma_v2::payload;
use tracing::Level;
use tracing::subscriber::with_default;

use crate::support::{CapturedLog, data, inputs, settings, value};

#[test]
fn sanitize_payload_keeps_a_json_object_as_it_is() {
    let fields = payload::sanitize_payload(r#"{"model": "gpt-4o"}"#, true);
    assert_eq!(fields, data(r#"{"model": "gpt-4o"}"#));
}

#[test]
fn sanitize_payload_returns_empty_for_text_that_is_not_json() {
    let log = CapturedLog::default();
    let fields = with_default(log.subscriber(Level::WARN), || {
        payload::sanitize_payload("not json", true)
    });
    assert!(fields.is_empty());
    assert!(log.text().contains("payload serialization failed"));
}

#[test]
fn sanitize_payload_stays_quiet_without_content() {
    let log = CapturedLog::default();
    let fields = with_default(log.subscriber(Level::WARN), || {
        payload::sanitize_payload("not json", false)
    });
    assert!(fields.is_empty());
    assert!(!log.text().contains("serialization"));
}

#[test]
fn sanitize_payload_returns_empty_for_a_json_list() {
    let log = CapturedLog::default();
    let fields = with_default(log.subscriber(Level::WARN), || {
        payload::sanitize_payload("[1, 2]", true)
    });
    assert!(fields.is_empty());
    let text = log.text();
    assert!(text.contains("non-object output"));
    assert!(text.contains("type=list"));
}

#[test]
fn an_empty_sanitized_object_logs_the_serialization_warning() {
    let log = CapturedLog::default();
    let fields = with_default(log.subscriber(Level::WARN), || {
        payload::sanitize_payload("{}", true)
    });
    assert!(fields.is_empty());
    assert!(log.text().contains("payload serialization failed"));
}

#[test]
fn the_wire_payload_serialization_keeps_every_scan_field() {
    let scanned = inputs();
    let sent_data = data(r#"{"model": "gpt-4o", "messages": [], "input": "x"}"#);
    let request: payload::ScanRequest<'_> = payload::ScanRequest {
        inputs: &scanned,
        request_data: &sent_data,
        input_type: GuardrailInputType::Request,
        log_target: None,
        application_id: Some(String::from("app-1")),
    };
    let config = settings("https://noma.example");
    let wire = payload::wire_payload(&payload::build(&request, &config));
    assert_eq!(
        wire.get("application_id"),
        Some(&GuardrailValue::Text(String::from("app-1")))
    );
    assert_eq!(
        wire.get("input_type"),
        Some(&GuardrailValue::Text(String::from("request")))
    );
    assert_eq!(
        wire.get("monitor_mode"),
        Some(&GuardrailValue::Boolean(false))
    );
    assert_eq!(wire.get("gateway_name"), None);
    let inputs_json = serde_json::to_string(&scanned).unwrap();
    assert_eq!(wire.get("inputs"), Some(&value(&inputs_json)));
    let Some(GuardrailValue::Object(sent)) = wire.get("request_data") else {
        panic!("the wire payload has no request_data object");
    };
    assert!(!sent.contains_key("messages"));
    assert!(!sent.contains_key("input"));
    assert_eq!(
        sent.get("model"),
        Some(&GuardrailValue::Text(String::from("gpt-4o")))
    );
}
