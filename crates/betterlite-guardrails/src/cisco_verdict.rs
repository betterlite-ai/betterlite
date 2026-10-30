use serde_json::{Map, Value};
use std::collections::BTreeMap;

const DECISION_FIELDS: &[&str] = &[
    "action",
    "allowed",
    "blocked",
    "safe",
    "is_safe",
    "decision",
    "verdict",
    "status",
    "score",
    "risk_score",
    "confidence",
    "categories",
    "classifications",
    "violations",
    "threats",
    "policies",
    "reason",
    "rules",
    "sanitized_text",
    "sanitizedText",
    "sanitized_payload",
];

pub fn has_decision_fields(payload: &Value) -> bool {
    let Value::Object(payload) = payload else {
        return false;
    };
    DECISION_FIELDS
        .iter()
        .any(|field| payload.contains_key(*field))
}

pub fn unwrap_verdict_envelope(response: &Value) -> Value {
    let Value::Object(response) = response else {
        return Value::Object(Map::new());
    };
    if has_decision_fields(&Value::Object(response.clone())) {
        return Value::Object(response.clone());
    }
    for key in ["result", "data", "inspection", "ai_defense", "aiDefense"] {
        if let Some(value) = response.get(key) {
            if has_decision_fields(value) {
                return value.clone();
            }
        }
    }
    if let Some(Value::Object(result)) = response.get("result") {
        for key in ["data", "inspection", "ai_defense", "aiDefense"] {
            if let Some(value) = result.get(key) {
                if has_decision_fields(value) {
                    return value.clone();
                }
            }
        }
    }
    Value::Object(response.clone())
}

pub fn normalize_action(raw_action: &str) -> String {
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
        return "block".to_owned();
    }
    if ["redact", "redacted", "sanitize", "sanitized", "mask"].contains(&normalized.as_str()) {
        return "redact".to_owned();
    }
    if ["allow", "allowed", "safe", "ok"].contains(&normalized.as_str()) {
        return "allow".to_owned();
    }
    tracing::warn!(
        action = raw_action,
        "unrecognized Cisco AI Defense action; treating as block"
    );
    "block".to_owned()
}

pub fn extract_jsonrpc_error(response: &Value) -> Option<Value> {
    let Value::Object(response) = response else {
        return None;
    };
    if let Some(error @ Value::Object(_)) = response.get("error") {
        return Some(error.clone());
    }
    if let Some(Value::Object(result)) = response.get("result") {
        if let Some(error @ Value::Object(_)) = result.get("error") {
            return Some(error.clone());
        }
    }
    None
}

pub fn extract_sanitized_text(response: &Value) -> Option<String> {
    for container in containers(response) {
        for key in ["sanitized_text", "sanitizedText"] {
            if let Some(Value::String(text)) = container.get(key) {
                if !text.is_empty() {
                    return Some(text.clone());
                }
            }
        }
    }
    None
}

pub fn extract_sanitized_messages(response: &Value) -> Option<Vec<Map<String, Value>>> {
    for container in containers(response) {
        for key in [
            "sanitized_messages",
            "sanitizedMessages",
            "modified_messages",
            "modifiedMessages",
        ] {
            if let Some(messages) = non_empty_objects(container.get(key)) {
                return Some(messages);
            }
        }
        for key in [
            "sanitized_payload",
            "sanitizedPayload",
            "modified_payload",
            "modifiedPayload",
        ] {
            if let Some(Value::Object(payload)) = container.get(key) {
                if let Some(messages) = non_empty_objects(payload.get("messages")) {
                    return Some(messages);
                }
            }
        }
    }
    None
}

pub fn extract_mcp_args(response: &Value) -> Option<Map<String, Value>> {
    for container in containers(response) {
        if let Some(arguments) = nested_arguments(container, "params") {
            return Some(arguments);
        }
        for key in [
            "sanitized_payload",
            "sanitizedPayload",
            "modified_payload",
            "modifiedPayload",
        ] {
            if let Some(Value::Object(payload)) = container.get(key) {
                if let Some(arguments) = nested_arguments(payload, "params") {
                    return Some(arguments);
                }
                if let Some(Value::Object(arguments)) = payload.get("arguments") {
                    if !arguments.is_empty() {
                        return Some(arguments.clone());
                    }
                }
            }
        }
        if let Some(Value::Object(arguments)) = container.get("arguments") {
            if !arguments.is_empty() {
                return Some(arguments.clone());
            }
        }
    }
    None
}

pub fn extract_masked_entity_count(rules: &Value) -> Option<BTreeMap<String, u64>> {
    let Value::Array(rules) = rules else {
        return None;
    };
    let mut counts = BTreeMap::new();
    for Value::Object(rule) in rules {
        let Some(Value::Array(entity_types)) = rule.get("entity_types") else {
            continue;
        };
        for Value::String(entity_type) in entity_types {
            let count = counts.entry(entity_type.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    (!counts.is_empty()).then_some(counts)
}

fn containers(response: &Value) -> Vec<&Map<String, Value>> {
    let Value::Object(response) = response else {
        return Vec::new();
    };
    let mut containers = vec![response];
    for key in ["result", "data"] {
        if let Some(Value::Object(container)) = response.get(key) {
            containers.push(container)
        }
    }
    containers
}

fn non_empty_objects(value: Option<&Value>) -> Option<Vec<Map<String, Value>>> {
    let Some(Value::Array(values)) = value else {
        return None;
    };
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .filter_map(|value| match value {
                Value::Object(object) => Some(object.clone()),
                _ => None,
            })
            .collect(),
    )
}

fn nested_arguments(container: &Map<String, Value>, key: &str) -> Option<Map<String, Value>> {
    let Some(Value::Object(params)) = container.get(key) else {
        return None;
    };
    let Some(Value::Object(arguments)) = params.get("arguments") else {
        return None;
    };
    (!arguments.is_empty()).then_some(arguments.clone())
}

mod tests;
