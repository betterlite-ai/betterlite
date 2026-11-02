#![cfg(test)]

use betterlite_config::sections::cisco_verdict_contracts::{
    FallbackAction, FlaggedAction, InspectionType,
};

use crate::cisco_configuration::records::{CiscoVerdict, Direction, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::api_error::ApiErrorOutcome;
use crate::cisco_verdict_enforcement::finalize::{
    FinalizeContext, FinalizeOutcome, ResponseSurface, apply_redaction_input,
    apply_redaction_output, finalize_inspection,
};

fn obj(json: &str) -> Fields {
    let Json::Object(fields) = Json::parse(json) else {
        panic!("expected a JSON object, got: {json}");
    };
    fields
}

fn finalize_context(
    surface: InspectionType,
    direction: Direction,
    on_flagged_action: FlaggedAction,
    fallback_on_error: FallbackAction,
) -> FinalizeContext<'static> {
    FinalizeContext {
        scan: ScanContext { surface, direction },
        duration_ms: 1.0,
        provider_name: "cisco",
        guardrail_name: "cisco-ai-defense",
        on_flagged_action,
        fallback_on_error,
    }
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

#[test]
fn finalize_inspection_allows_when_action_is_missing() {
    let inspect_response = Json::parse("{}");
    let mut request_data = obj("{}");
    let finalize = finalize_context(
        InspectionType::Chat,
        Direction::Input,
        FlaggedAction::Block,
        FallbackAction::Block,
    );
    let outcome = finalize_inspection(
        &inspect_response,
        &mut request_data,
        ResponseSurface::None,
        &finalize,
    );
    assert!(matches!(outcome, FinalizeOutcome::Allow));
}

#[test]
fn finalize_inspection_blocks_on_block_action_with_block_policy() {
    let inspect_response = Json::parse(r#"{"action": "block", "severity": "high"}"#);
    let mut request_data = obj("{}");
    let finalize = finalize_context(
        InspectionType::Chat,
        Direction::Input,
        FlaggedAction::Block,
        FallbackAction::Block,
    );
    let outcome = finalize_inspection(
        &inspect_response,
        &mut request_data,
        ResponseSurface::None,
        &finalize,
    );
    let FinalizeOutcome::Blocked(payload) = outcome else {
        panic!("expected Blocked");
    };
    assert_eq!(payload.action, "block");
    assert_eq!(payload.severity, Some("high".to_owned()));
}

#[test]
fn finalize_inspection_monitors_on_block_action_with_monitor_policy() {
    let inspect_response = Json::parse(r#"{"action": "block"}"#);
    let mut request_data = obj("{}");
    let finalize = finalize_context(
        InspectionType::Chat,
        Direction::Input,
        FlaggedAction::Monitor,
        FallbackAction::Block,
    );
    let outcome = finalize_inspection(
        &inspect_response,
        &mut request_data,
        ResponseSurface::None,
        &finalize,
    );
    assert!(matches!(outcome, FinalizeOutcome::Monitored));
}

#[test]
fn finalize_inspection_redacts_when_rewrite_succeeds() {
    let inspect_response = Json::parse(r#"{"action": "redact", "sanitized_text": "clean"}"#);
    let mut request_data = obj(r#"{"instructions": "secret"}"#);
    let finalize = finalize_context(
        InspectionType::Chat,
        Direction::Input,
        FlaggedAction::Block,
        FallbackAction::Block,
    );
    let outcome = finalize_inspection(
        &inspect_response,
        &mut request_data,
        ResponseSurface::None,
        &finalize,
    );
    assert!(matches!(outcome, FinalizeOutcome::Redacted(_)));
    assert_eq!(
        request_data.get("instructions").map(|raw| raw.get()),
        Some("\"clean\"")
    );
}

#[test]
fn finalize_inspection_falls_through_to_flagged_action_when_rewrite_fails() {
    let inspect_response = Json::parse(r#"{"action": "redact"}"#);
    let mut request_data = obj("{}");
    let finalize = finalize_context(
        InspectionType::Chat,
        Direction::Input,
        FlaggedAction::Monitor,
        FallbackAction::Block,
    );
    let outcome = finalize_inspection(
        &inspect_response,
        &mut request_data,
        ResponseSurface::None,
        &finalize,
    );
    assert!(matches!(outcome, FinalizeOutcome::Monitored));
}

#[test]
fn finalize_inspection_handles_a_jsonrpc_error_envelope_as_an_api_error() {
    let inspect_response = Json::parse(r#"{"error": {"code": -1, "message": "boom"}}"#);
    let mut request_data = obj("{}");
    let finalize = finalize_context(
        InspectionType::Mcp,
        Direction::Input,
        FlaggedAction::Block,
        FallbackAction::Allow,
    );
    let outcome = finalize_inspection(
        &inspect_response,
        &mut request_data,
        ResponseSurface::None,
        &finalize,
    );
    let FinalizeOutcome::ApiError(ApiErrorOutcome::Unscanned) = outcome else {
        panic!("expected an unscanned API error outcome");
    };
}

#[test]
fn apply_redaction_input_dispatches_to_mcp_for_mcp_surface() {
    let mut request_data = obj(r#"{"arguments": {"question": "secret"}}"#);
    let verdict = CiscoVerdict {
        sanitized_text: Some("redacted".to_owned()),
        ..empty_verdict()
    };
    let outcome = apply_redaction_input(&mut request_data, InspectionType::Mcp, &verdict);
    assert!(outcome.applied);
}

#[test]
fn apply_redaction_input_dispatches_to_chat_for_chat_surface() {
    let mut request_data = obj(r#"{"instructions": "old"}"#);
    let verdict = CiscoVerdict {
        sanitized_text: Some("redacted".to_owned()),
        ..empty_verdict()
    };
    let outcome = apply_redaction_input(&mut request_data, InspectionType::Chat, &verdict);
    assert!(outcome.applied);
    assert_eq!(
        request_data.get("instructions").map(|raw| raw.get()),
        Some("\"redacted\"")
    );
}

#[test]
fn apply_redaction_output_returns_unapplied_for_a_mismatched_surface() {
    let verdict = CiscoVerdict {
        sanitized_text: Some("redacted".to_owned()),
        ..empty_verdict()
    };
    let outcome = apply_redaction_output(ResponseSurface::None, InspectionType::Mcp, &verdict);
    assert!(!outcome.applied);
    assert!(outcome.mcp_output_update.is_none());
}

#[test]
fn apply_redaction_output_rewrites_the_mcp_response() {
    let response = Json::parse(r#"{"content": [{"type": "text", "text": "old"}]}"#);
    let verdict = CiscoVerdict {
        sanitized_text: Some("redacted".to_owned()),
        ..empty_verdict()
    };
    let outcome = apply_redaction_output(
        ResponseSurface::McpResponse(&response),
        InspectionType::Mcp,
        &verdict,
    );
    assert!(outcome.applied);
    assert!(outcome.mcp_output_update.is_some());
}

#[test]
fn apply_redaction_output_rewrites_the_chat_response() {
    let mut response = obj(r#"{"choices": [{"message": {"content": "old"}}]}"#);
    let verdict = CiscoVerdict {
        sanitized_text: Some("redacted".to_owned()),
        ..empty_verdict()
    };
    let outcome = apply_redaction_output(
        ResponseSurface::ChatResponse(&mut response),
        InspectionType::Chat,
        &verdict,
    );
    assert!(outcome.applied);
}
