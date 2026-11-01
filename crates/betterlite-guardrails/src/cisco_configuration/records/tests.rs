#![cfg(test)]

use betterlite_config::sections::cisco_verdict_contracts::InspectionType;
use indexmap::IndexMap;

use crate::cisco_configuration::records::{CiscoVerdict, Direction, ScanContext};
use crate::cisco_verdict::Action;

#[test]
fn scan_context_holds_surface_and_direction() {
    let context = ScanContext {
        surface: InspectionType::Mcp,
        direction: Direction::Output,
    };
    assert_eq!(context.surface, InspectionType::Mcp);
    assert_eq!(context.direction, Direction::Output);
}

#[test]
fn scan_context_is_copy() {
    let context = ScanContext {
        surface: InspectionType::Chat,
        direction: Direction::Input,
    };
    let copied = context;
    assert_eq!(context.surface, copied.surface);
}

#[test]
fn cisco_verdict_holds_every_field_cisco_can_return() {
    let verdict = CiscoVerdict {
        is_safe: Some(false),
        classifications: vec!["pii".to_owned()],
        severity: Some("high".to_owned()),
        rules: vec![IndexMap::new()],
        explanation: Some("matched a rule".to_owned()),
        event_id: Some("evt-1".to_owned()),
        action: Some(Action::Redact),
        sanitized_text: Some("[redacted]".to_owned()),
        sanitized_messages: Some(vec![IndexMap::new()]),
        sanitized_mcp_arguments: Some(IndexMap::new()),
    };
    assert_eq!(verdict.is_safe, Some(false));
    assert_eq!(verdict.classifications, vec!["pii".to_owned()]);
    assert_eq!(verdict.severity, Some("high".to_owned()));
    assert_eq!(verdict.rules.len(), 1);
    assert_eq!(verdict.explanation, Some("matched a rule".to_owned()));
    assert_eq!(verdict.event_id, Some("evt-1".to_owned()));
    assert_eq!(verdict.action, Some(Action::Redact));
    assert_eq!(verdict.sanitized_text, Some("[redacted]".to_owned()));
    assert_eq!(verdict.sanitized_messages.expect("messages").len(), 1);
    assert!(verdict.sanitized_mcp_arguments.is_some());
}

#[test]
fn cisco_verdict_defaults_optional_fields_to_none_when_absent() {
    let verdict = CiscoVerdict {
        is_safe: None,
        classifications: Vec::new(),
        severity: None,
        rules: Vec::new(),
        explanation: None,
        event_id: None,
        action: None,
        sanitized_text: None,
        sanitized_messages: None,
        sanitized_mcp_arguments: None,
    };
    assert_eq!(verdict.is_safe, None);
    assert!(verdict.classifications.is_empty());
    assert_eq!(verdict.action, None);
    assert!(verdict.sanitized_messages.is_none());
    assert!(verdict.sanitized_mcp_arguments.is_none());
}
