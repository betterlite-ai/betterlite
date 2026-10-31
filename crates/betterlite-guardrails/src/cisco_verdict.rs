use std::collections::BTreeMap;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

pub mod decision;

#[must_use]
pub fn unwrap_verdict_envelope(response: &Json) -> Json {
    let Json::Object(fields) = response else {
        return Json::Object(Fields::new());
    };
    if decision::has_decision_fields(response) {
        return response.clone();
    }
    for key in ["result", "data", "inspection", "ai_defense", "aiDefense"] {
        let value = Json::field(fields, key);
        if decision::has_decision_fields(&value) {
            return value;
        }
    }
    if let Json::Object(result) = Json::field(fields, "result") {
        for key in ["data", "inspection", "ai_defense", "aiDefense"] {
            let value = Json::field(&result, key);
            if decision::has_decision_fields(&value) {
                return value;
            }
        }
    }
    response.clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Block,
    Redact,
    Allow,
}

impl Action {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Redact => "redact",
            Self::Allow => "allow",
        }
    }
}

#[must_use]
pub fn normalize_action(raw_action: &str) -> Action {
    let normalized = raw_action.trim().to_ascii_lowercase();
    if [
        "deny",
        "denied",
        "block",
        "blocked",
        "reject",
        "rejected",
        "unsafe",
        "malicious",
    ]
    .contains(&normalized.as_str())
    {
        return Action::Block;
    }
    if ["redact", "redacted", "sanitize", "sanitized", "mask"].contains(&normalized.as_str()) {
        return Action::Redact;
    }
    if ["allow", "allowed", "safe", "ok"].contains(&normalized.as_str()) {
        return Action::Allow;
    }
    tracing::warn!(
        action = raw_action,
        "unrecognized Cisco AI Defense action; treating as block"
    );
    Action::Block
}

#[must_use]
pub fn extract_jsonrpc_error(response: &Json) -> Option<Json> {
    let Json::Object(fields) = response else {
        return None;
    };
    let error = Json::field(fields, "error");
    if let Json::Object(_) = error {
        return Some(error);
    }
    if let Json::Object(result) = Json::field(fields, "result") {
        let nested_error = Json::field(&result, "error");
        if let Json::Object(_) = nested_error {
            return Some(nested_error);
        }
    }
    None
}

#[must_use]
pub fn extract_sanitized_text(response: &Json) -> Option<String> {
    let Json::Object(fields) = response else {
        return None;
    };
    if let Some(text) = text_in(fields) {
        return Some(text);
    }
    if let Json::Object(result) = Json::field(fields, "result") {
        return text_in(&result);
    }
    None
}

#[must_use]
pub fn extract_sanitized_messages(response: &Json) -> Option<Vec<Fields>> {
    let response_containers = containers(response);
    response_containers
        .into_iter()
        .find_map(|container| message_in(&container))
}

fn text_in(container: &Fields) -> Option<String> {
    let text_keys = ["sanitized_text", "sanitizedText"];
    text_keys
        .iter()
        .find_map(|key| match Json::field(container, key) {
            Json::Text(text) if !text.is_empty() => Some(text),
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => None,
        })
}

fn message_in(container: &Fields) -> Option<Vec<Fields>> {
    let direct = [
        "sanitized_messages",
        "sanitizedMessages",
        "modified_messages",
        "modifiedMessages",
    ];
    if let Some(messages) = direct
        .iter()
        .find_map(|key| non_empty_objects(&Json::field(container, key)))
    {
        return Some(messages);
    }
    payload_objects(container)
        .into_iter()
        .find_map(|payload| non_empty_objects(&Json::field(&payload, "messages")))
}

#[must_use]
pub fn extract_mcp_args(response: &Json) -> Option<Fields> {
    let response_containers = containers(response);
    response_containers
        .into_iter()
        .find_map(|container| arguments_in(&container))
}

fn arguments_in(container: &Fields) -> Option<Fields> {
    if let Some(arguments) = nested_arguments(container, "params") {
        return Some(arguments);
    }
    for payload in payload_objects(container) {
        if let Some(arguments) = nested_arguments(&payload, "params") {
            return Some(arguments);
        }
        if let Some(arguments) = non_empty_object(&Json::field(&payload, "arguments")) {
            return Some(arguments);
        }
    }
    None
}

fn payload_objects(container: &Fields) -> Vec<Fields> {
    let payload_keys = [
        "sanitized_payload",
        "sanitizedPayload",
        "modified_payload",
        "modifiedPayload",
    ];
    payload_keys
        .iter()
        .filter_map(|key| match Json::field(container, key) {
            Json::Object(payload) => Some(payload),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

#[must_use]
pub fn extract_masked_entity_count(input: &Json) -> Option<BTreeMap<String, u64>> {
    let Json::List(rules) = input else {
        return None;
    };
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for raw_rule in rules {
        let parsed_rule = Json::parse(raw_rule.get());
        let Json::Object(rule_fields) = parsed_rule else {
            continue;
        };
        let Json::List(entity_types) = Json::field(&rule_fields, "entity_types") else {
            continue;
        };
        for raw_entity_type in entity_types {
            let Json::Text(entity_type) = Json::parse(raw_entity_type.get()) else {
                continue;
            };
            let count = counts.entry(entity_type.clone()).or_insert(0_u64);
            *count = count.saturating_add(1);
        }
    }
    (!counts.is_empty()).then_some(counts)
}

fn containers(response: &Json) -> Vec<Fields> {
    let Json::Object(fields) = response else {
        return Vec::new();
    };
    let mut containers = vec![fields.clone()];
    for key in ["result", "data"] {
        if let Json::Object(container) = Json::field(fields, key) {
            containers.push(container);
        }
    }
    containers
}

fn non_empty_objects(value: &Json) -> Option<Vec<Fields>> {
    let Json::List(values) = value else {
        return None;
    };
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .filter_map(|raw_value| match Json::parse(raw_value.get()) {
                Json::Object(object) => Some(object),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
                    None
                }
            })
            .collect(),
    )
}

fn non_empty_object(value: &Json) -> Option<Fields> {
    let Json::Object(object) = value else {
        return None;
    };
    (!object.is_empty()).then_some(object.clone())
}

fn nested_arguments(container: &Fields, key: &str) -> Option<Fields> {
    let Json::Object(params) = Json::field(container, key) else {
        return None;
    };
    let Json::Object(arguments) = Json::field(&params, "arguments") else {
        return None;
    };
    (!arguments.is_empty()).then_some(arguments)
}

mod tests;
