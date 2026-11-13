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
        let value = Json::parse(fields.get(key).map_or("null", |raw| raw.get()));
        if decision::has_decision_fields(&value) {
            return value;
        }
    }
    if let Json::Object(result) = Json::parse(fields.get("result").map_or("null", |raw| raw.get()))
    {
        for key in ["data", "inspection", "ai_defense", "aiDefense"] {
            let value = Json::parse(result.get(key).map_or("null", |raw| raw.get()));
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
    let error = Json::parse(fields.get("error").map_or("null", |raw| raw.get()));
    if let Json::Object(_) = error {
        return Some(error);
    }
    if let Json::Object(result) = Json::parse(fields.get("result").map_or("null", |raw| raw.get()))
    {
        let nested_error = Json::parse(result.get("error").map_or("null", |raw| raw.get()));
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
    if let Json::Object(result) = Json::parse(fields.get("result").map_or("null", |raw| raw.get()))
    {
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
    for key in ["sanitized_text", "sanitizedText"] {
        let Some(raw) = container.get(key) else {
            continue;
        };
        if let Json::Text(text) = Json::parse(raw.get())
            && !text.is_empty()
        {
            return Some(text);
        }
    }
    None
}

fn message_in(container: &Fields) -> Option<Vec<Fields>> {
    let direct = [
        "sanitized_messages",
        "sanitizedMessages",
        "modified_messages",
        "modifiedMessages",
    ];
    if let Some(messages) = direct.iter().find_map(|&key| {
        non_empty_objects(&Json::parse(
            container.get(key).map_or("null", |raw| raw.get()),
        ))
    }) {
        return Some(messages);
    }
    payload_objects(container).into_iter().find_map(|payload| {
        non_empty_objects(&Json::parse(
            payload.get("messages").map_or("null", |raw| raw.get()),
        ))
    })
}

#[must_use]
pub fn extract_mcp_args(response: &Json) -> Option<Fields> {
    for container in containers(response) {
        if let Some(arguments) = nested_arguments(&container, "params") {
            return Some(arguments);
        }
        for payload in payload_objects(&container) {
            if let Some(arguments) = nested_arguments(&payload, "params") {
                return Some(arguments);
            }
            if let Some(raw) = payload.get("arguments")
                && let Some(arguments) = non_empty_object(&Json::parse(raw.get()))
            {
                return Some(arguments);
            }
        }
    }
    None
}

fn payload_objects(container: &Fields) -> Vec<Fields> {
    let mut payloads = Vec::new();
    for key in [
        "sanitized_payload",
        "sanitizedPayload",
        "modified_payload",
        "modifiedPayload",
    ] {
        if let Some(raw) = container.get(key)
            && let Json::Object(payload) = Json::parse(raw.get())
        {
            payloads.push(payload);
        }
    }
    payloads
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
        let Json::List(entity_types) = Json::parse(
            rule_fields
                .get("entity_types")
                .map_or("null", |raw| raw.get()),
        ) else {
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
        if let Json::Object(container) =
            Json::parse(fields.get(key).map_or("null", |raw| raw.get()))
        {
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
    let Json::Object(params) = Json::parse(container.get(key).map_or("null", |raw| raw.get()))
    else {
        return None;
    };
    let Json::Object(arguments) =
        Json::parse(params.get("arguments").map_or("null", |raw| raw.get()))
    else {
        return None;
    };
    (!arguments.is_empty()).then_some(arguments)
}

mod tests;
