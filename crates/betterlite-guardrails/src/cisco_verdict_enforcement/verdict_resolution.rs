use serde_json::value::RawValue;

use crate::cisco_configuration::records::CiscoVerdict;
use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;
use crate::cisco_verdict::{self, Action};

// Mirrors LiteLLM's fn _finalize_inspection verdict construction, with action resolved by fn _normalize_action (missing or blank allows, an unrecognized string blocks).
#[must_use]
pub fn resolve_verdict(inspect_response: &Json) -> CiscoVerdict {
    let unwrapped = cisco_verdict::unwrap_verdict_envelope(inspect_response);
    let Json::Object(fields) = &unwrapped else {
        return allow_verdict();
    };
    CiscoVerdict {
        is_safe: bool_field(fields, "is_safe"),
        classifications: classifications_from(fields),
        severity: text_field(fields, "severity"),
        rules: rules_from(fields),
        explanation: text_field(fields, "explanation"),
        event_id: text_field(fields, "event_id"),
        action: Some(action_from(fields)),
        sanitized_text: cisco_verdict::extract_sanitized_text(&unwrapped),
        sanitized_messages: cisco_verdict::extract_sanitized_messages(&unwrapped),
        sanitized_mcp_arguments: cisco_verdict::extract_mcp_args(&unwrapped),
    }
}

const fn allow_verdict() -> CiscoVerdict {
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

fn bool_field(fields: &Fields, key: &str) -> Option<bool> {
    match Json::field(fields, key) {
        Json::Bool(value) => Some(value),
        Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn text_field(fields: &Fields, key: &str) -> Option<String> {
    match Json::field(fields, key) {
        Json::Text(value) => Some(value),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn list_field(fields: &Fields, key: &str) -> Vec<Box<RawValue>> {
    match Json::field(fields, key) {
        Json::List(items) => items,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            Vec::new()
        }
    }
}

fn classifications_from(fields: &Fields) -> Vec<String> {
    let listed = text_list(&list_field(fields, "classifications"));
    if listed.is_empty() {
        single_classification(fields)
    } else {
        listed
    }
}

fn single_classification(fields: &Fields) -> Vec<String> {
    match Json::field(fields, "classification") {
        Json::Text(value) if !value.is_empty() => vec![value],
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => Vec::new(),
    }
}

fn text_list(items: &[Box<RawValue>]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match Json::parse(item.get()) {
            Json::Text(value) => Some(value),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

fn rules_from(fields: &Fields) -> Vec<Fields> {
    let items = list_field(fields, "rules");
    items
        .iter()
        .filter_map(|item| match Json::parse(item.get()) {
            Json::Object(rule) => Some(rule),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

fn action_from(fields: &Fields) -> Action {
    match Json::field(fields, "action") {
        Json::Text(raw_action) if !raw_action.trim().is_empty() => {
            cisco_verdict::normalize_action(&raw_action)
        }
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => Action::Allow,
    }
}

mod tests;
