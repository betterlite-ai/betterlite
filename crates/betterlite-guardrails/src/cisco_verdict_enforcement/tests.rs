#![cfg(test)]

use std::sync::Arc;
use std::{env, fs, process};

use betterlite_config::sections::cisco_verdict_contracts::{FallbackAction, InspectionType};
use rstest::rstest;
use tracing::subscriber::set_default;

use crate::cisco_configuration::records::{CiscoVerdict, Direction, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use crate::cisco_verdict_enforcement::block_payload::GuardrailException;
use crate::cisco_verdict_enforcement::finalize::ResponseSurface;
use crate::cisco_verdict_enforcement::{
    apply_redaction, build_block_payload, handle_api_error, http_exception_to_error_obj,
    log_decision, strip_sensitive_keys,
};

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn empty_verdict() -> CiscoVerdict {
    CiscoVerdict {
        is_safe: None,
        classifications: Vec::new(),
        severity: None,
        rules: Vec::new(),
        explanation: None,
        event_id: None,
        action: Some(Action::Allow),
        sanitized_text: None,
        sanitized_messages: None,
        sanitized_mcp_arguments: None,
    }
}

fn context() -> ScanContext {
    ScanContext {
        surface: InspectionType::Chat,
        direction: Direction::Input,
    }
}

#[test]
fn strip_sensitive_keys_drops_underscore_prefixed_and_redacted_keys() {
    let fields = obj(
        r#"{"_internal": 1, "raw_request": "secret", "sanitizedPayload": "x", "model": "gpt"}"#,
    );
    let stripped = strip_sensitive_keys(&fields);
    assert!(!stripped.contains_key("_internal"));
    assert!(!stripped.contains_key("raw_request"));
    assert!(!stripped.contains_key("sanitizedPayload"));
    assert!(stripped.contains_key("model"));
}

#[test]
fn strip_sensitive_keys_recurses_into_nested_objects() {
    let fields = obj(r#"{"result": {"raw_request": "secret", "is_safe": true}}"#);
    let stripped = strip_sensitive_keys(&fields);
    let Json::Object(nested) = Json::field(&stripped, "result") else {
        panic!("result missing");
    };
    assert!(!nested.contains_key("raw_request"));
    assert!(nested.contains_key("is_safe"));
}

#[test]
fn build_block_payload_carries_the_verdict_audit_fields() {
    let mut verdict = empty_verdict();
    verdict.classifications = vec!["PII".to_owned()];
    verdict.severity = Some("high".to_owned());
    verdict.event_id = Some("evt-1".to_owned());
    let payload = build_block_payload(context(), &verdict, "cisco", "cisco-ai-defense");
    assert_eq!(payload.action, "block");
    assert_eq!(payload.surface, "chat");
    assert_eq!(payload.direction, "input");
    assert_eq!(payload.provider, "cisco");
    assert_eq!(payload.guardrail, "cisco-ai-defense");
    assert_eq!(payload.classifications, vec!["PII".to_owned()]);
    assert_eq!(payload.severity, Some("high".to_owned()));
    assert_eq!(payload.event_id, Some("evt-1".to_owned()));
}

#[rstest]
#[case::cisco_shaped_detail(
    400,
    r#"{"error": "Blocked", "message": "nope", "guardrail": "cisco"}"#,
    "\"nope\"",
    "\"cisco\""
)]
#[case::bare_detail(
    503,
    r#"{"error": "unavailable"}"#,
    "\"unavailable\"",
    "\"cisco-ai-defense\""
)]
fn http_exception_to_error_obj_fills_defaults_only_when_missing(
    #[case] status_code: u16,
    #[case] detail: &str,
    #[case] expected_message: &str,
    #[case] expected_guardrail: &str,
) {
    let exception = GuardrailException {
        status_code,
        detail: Json::parse(detail),
    };
    let error_obj = http_exception_to_error_obj(&exception, "cisco-ai-defense");
    assert_eq!(
        error_obj.get("message").map(|raw| raw.get()),
        Some(expected_message)
    );
    assert_eq!(
        error_obj.get("guardrail").map(|raw| raw.get()),
        Some(expected_guardrail)
    );
}

#[test]
fn http_exception_to_error_obj_fills_a_default_code_for_a_bare_detail() {
    let exception = GuardrailException {
        status_code: 503,
        detail: Json::parse(r#"{"error": "unavailable"}"#),
    };
    let error_obj = http_exception_to_error_obj(&exception, "cisco-ai-defense");
    assert_eq!(error_obj.get("code").map(|raw| raw.get()), Some("503"));
}

#[test]
fn log_decision_logs_at_info_for_an_allowed_verdict() {
    let log_path =
        env::temp_dir().join(format!("betterlite-cisco-decision-allow-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let guard = set_default(subscriber);
    log_decision(
        context(),
        &empty_verdict(),
        12.34,
        "cisco-ai-defense",
        &obj("{}"),
    );
    drop(guard);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("CISCO_AI_DEFENSE_DECISION"));
    assert!(written.contains("\"action\":\"allow\""));
    assert!(written.contains("\"duration_ms\":12.3"));
}

#[test]
fn log_decision_logs_at_warn_for_a_blocked_verdict() {
    let log_path =
        env::temp_dir().join(format!("betterlite-cisco-decision-block-{}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(Arc::new(fs::File::create(&log_path).expect("log file")))
        .finish();
    let mut verdict = empty_verdict();
    verdict.action = Some(Action::Block);
    let guard = set_default(subscriber);
    log_decision(context(), &verdict, 1.0, "cisco-ai-defense", &obj("{}"));
    drop(guard);
    let written = fs::read_to_string(&log_path).expect("log contents");
    fs::remove_file(&log_path).expect("remove log");
    assert!(written.contains("WARN"));
    assert!(written.contains("\"action\":\"block\""));
}

#[test]
fn apply_redaction_dispatches_on_direction() {
    let mut request_data = obj(r#"{"instructions": "old"}"#);
    let verdict = CiscoVerdict {
        sanitized_text: Some("redacted".to_owned()),
        ..empty_verdict()
    };
    let outcome = apply_redaction(
        &mut request_data,
        ResponseSurface::None,
        context(),
        &verdict,
    );
    assert!(outcome.applied);
}

#[test]
fn handle_api_error_unscans_when_fallback_is_allow() {
    let outcome = handle_api_error(InspectionType::Chat, "timeout", FallbackAction::Allow);
    assert!(matches!(outcome, ApiErrorOutcome::Unscanned));
}

#[test]
fn handle_api_error_blocks_when_fallback_is_block() {
    let outcome = handle_api_error(InspectionType::Chat, "timeout", FallbackAction::Block);
    let ApiErrorOutcome::Blocked(detail) = outcome else {
        panic!("expected Blocked");
    };
    assert_eq!(detail.error_type, "timeout");
}
