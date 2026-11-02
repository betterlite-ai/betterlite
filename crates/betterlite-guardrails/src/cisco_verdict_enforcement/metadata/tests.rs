#![cfg(test)]

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;

use crate::cisco_configuration::records::{CiscoVerdict, Direction, ScanContext};
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::Action;
use crate::cisco_verdict_enforcement::metadata::{
    sanitize_response_for_logging, stash_verdict_on_request,
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
fn stash_verdict_on_request_creates_metadata_when_absent() {
    let mut request_data = obj("{}");
    stash_verdict_on_request(&mut request_data, context(), &empty_verdict());
    let Json::Object(metadata) = Json::field(&request_data, "metadata") else {
        panic!("metadata missing");
    };
    assert!(metadata.contains_key("cisco_ai_defense_chat_input_is_safe"));
}

#[test]
fn stash_verdict_on_request_always_sets_is_safe_even_when_none() {
    let mut request_data = obj("{}");
    stash_verdict_on_request(&mut request_data, context(), &empty_verdict());
    let Json::Object(metadata) = Json::field(&request_data, "metadata") else {
        panic!("metadata missing");
    };
    assert_eq!(
        metadata
            .get("cisco_ai_defense_chat_input_is_safe")
            .map(|raw| raw.get()),
        Some("null")
    );
}

#[test]
fn stash_verdict_on_request_sets_conditional_fields_when_present() {
    let mut request_data = obj("{}");
    let mut verdict = empty_verdict();
    verdict.classifications = vec!["PII".to_owned()];
    verdict.severity = Some("high".to_owned());
    verdict.event_id = Some("evt-1".to_owned());
    verdict.rules = vec![obj(r#"{"rule_name": "pii"}"#)];
    stash_verdict_on_request(&mut request_data, context(), &verdict);
    let Json::Object(metadata) = Json::field(&request_data, "metadata") else {
        panic!("metadata missing");
    };
    assert!(metadata.contains_key("cisco_ai_defense_chat_input_action"));
    assert!(metadata.contains_key("cisco_ai_defense_chat_input_classifications"));
    assert!(metadata.contains_key("cisco_ai_defense_chat_input_severity"));
    assert!(metadata.contains_key("cisco_ai_defense_chat_input_rules"));
    assert!(metadata.contains_key("cisco_ai_defense_chat_input_event_id"));
}

#[test]
fn stash_verdict_on_request_skips_conditional_fields_when_empty() {
    let mut request_data = obj("{}");
    stash_verdict_on_request(&mut request_data, context(), &empty_verdict());
    let Json::Object(metadata) = Json::field(&request_data, "metadata") else {
        panic!("metadata missing");
    };
    assert!(!metadata.contains_key("cisco_ai_defense_chat_input_classifications"));
    assert!(!metadata.contains_key("cisco_ai_defense_chat_input_severity"));
    assert!(!metadata.contains_key("cisco_ai_defense_chat_input_rules"));
    assert!(!metadata.contains_key("cisco_ai_defense_chat_input_event_id"));
}

#[test]
fn stash_verdict_on_request_leaves_non_object_metadata_alone() {
    let mut request_data = obj(r#"{"metadata": "already-set"}"#);
    stash_verdict_on_request(&mut request_data, context(), &empty_verdict());
    assert_eq!(
        request_data.get("metadata").map(|raw| raw.get()),
        Some("\"already-set\"")
    );
}

#[test]
fn sanitize_response_for_logging_strips_sensitive_keys_and_tags_surface_and_action() {
    let response = Json::parse(r#"{"is_safe": true, "raw_request": "secret", "_internal": 1}"#);
    let sanitized =
        sanitize_response_for_logging(&response, InspectionType::Mcp, Some(Action::Block));
    assert!(!sanitized.contains_key("raw_request"));
    assert!(!sanitized.contains_key("_internal"));
    assert_eq!(
        sanitized.get("surface").map(|raw| raw.get()),
        Some("\"mcp\"")
    );
    assert_eq!(
        sanitized.get("action").map(|raw| raw.get()),
        Some("\"block\"")
    );
}

#[test]
fn sanitize_response_for_logging_tags_a_non_object_response() {
    let response = Json::parse("[1, 2, 3]");
    let sanitized = sanitize_response_for_logging(&response, InspectionType::Chat, None);
    assert_eq!(
        sanitized.get("surface").map(|raw| raw.get()),
        Some("\"chat\"")
    );
    assert!(!sanitized.contains_key("action"));
}
