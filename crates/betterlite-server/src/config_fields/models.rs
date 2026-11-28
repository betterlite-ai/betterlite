use betterlite_config::sections::settings::JsonValue;
use betterlite_pyhost::json_values::JsonObject;

use super::validation::{Check, is_integer};

mod rules;

use rules::{
    CLIENT_RULES, DYNAMO_RULES, ENDPOINT_RULES, GUARDRAIL_RULES, HEADER_KEYS, HEADER_RULES,
    Presence, Rule, SPEND_KEYS, SPEND_RULES, STAGGER_KEYS, STAGGER_RULES, VERSIONS,
};

// one declared field: how a present value is checked, and whether it may be absent or null
fn check_fields(entries: &JsonObject, rules: &[Rule]) -> bool {
    rules
        .iter()
        .all(|(name, check, presence)| check_field(entries.get(*name), *check, *presence))
}

fn check_field(value: Option<&JsonValue>, check: Check, presence: Presence) -> bool {
    match (value, presence) {
        (None, Presence::Required)
        | (Some(JsonValue::Null), Presence::Optional | Presence::Required) => false,
        (None, Presence::Optional | Presence::Nullable)
        | (Some(JsonValue::Null), Presence::Nullable) => true,
        (Some(present), Presence::Optional | Presence::Nullable | Presence::Required) => {
            check(present)
        }
    }
}

// an object holding the rules' fields
fn object_rules(value: &JsonValue, rules: &[Rule]) -> bool {
    value
        .as_object()
        .is_some_and(|entries| check_fields(entries, rules))
}

// an object that also carries no key outside the given names
fn known_object(value: &JsonValue, names: &[&str], rules: &[Rule]) -> bool {
    value.as_object().is_some_and(|entries| {
        entries.keys().all(|key| names.contains(&key.as_str())) && check_fields(entries, rules)
    })
}

// PassThroughGenericEndpoint, one entry of pass_through_endpoints
#[must_use]
pub fn endpoints(value: &JsonValue) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    items.iter().all(|item| object_rules(item, &ENDPOINT_RULES))
}

fn is_guardrails(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    entries
        .values()
        .all(|entry| entry.is_null() || object_rules(entry, &GUARDRAIL_RULES))
}

// UserHeaderMapping forbids unknown keys
#[must_use]
pub fn header_mappings(value: &JsonValue) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    items
        .iter()
        .all(|item| known_object(item, &HEADER_KEYS, &HEADER_RULES))
}

// MCPAdvertisedVersions: the four legacy revisions, at least one
#[must_use]
pub fn advertised_versions(value: &JsonValue) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    items
        .iter()
        .all(|item| matches!(item.as_str(), Some(text) if VERSIONS.contains(&text)))
}

// MCPAllowedClient: an alias and a value, each a non-empty string
#[must_use]
pub fn allowed_clients(value: &JsonValue) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    items.iter().all(|item| object_rules(item, &CLIENT_RULES))
}

// DynamoDBArgs
#[must_use]
pub fn database_args(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    check_fields(entries, &DYNAMO_RULES)
}

// ScheduledJobStaggerSettings forbids unknown keys and a window_seconds below zero
#[must_use]
pub fn stagger(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    entries
        .keys()
        .all(|key| STAGGER_KEYS.contains(&key.as_str()))
        && check_fields(entries, &STAGGER_RULES)
}

// SpendCaptureRateCheckSettings forbids unknown keys, a threshold outside (0, 1] and a lookback outside 1..=180
#[must_use]
pub fn spend_capture(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    let threshold = entries
        .get("threshold")
        .is_none_or(|entry| entry.as_f64().is_some_and(|rate| rate > 0.0 && rate <= 1.0));
    let lookback = entries.get("lookback_days").is_none_or(|entry| {
        is_integer(entry)
            && entry
                .as_f64()
                .is_some_and(|days| (1.0..=180.0).contains(&days))
    });
    entries.keys().all(|key| SPEND_KEYS.contains(&key.as_str()))
        && check_fields(entries, &SPEND_RULES)
        && threshold
        && lookback
}

fn non_empty_openai(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty() && items.iter().all(|item| item.as_str() == Some("openai"))
    })
}
