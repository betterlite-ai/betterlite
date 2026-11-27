use betterlite_config::sections::settings::JsonValue;
use betterlite_pyhost::json_values::JsonObject;

use crate::plugin_routes::config::PluginConfig;

use super::validation::{Check, is_integer};

mod rules;

use rules::{
    CLIENT_RULES, DYNAMO_RULES, ENDPOINT_RULES, GUARDRAIL_RULES, HEADER_KEYS, HEADER_RULES,
    Presence, REDIS_NODE_RULES, REDIS_RULES, Rule, SPEND_KEYS, SPEND_RULES, STAGGER_KEYS,
    STAGGER_RULES, VERSIONS,
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

fn only_known(entries: &JsonObject, names: &[&str]) -> bool {
    entries.keys().all(|key| names.contains(&key.as_str()))
}

// PluginConfig's own deserializer: name and url strings, display_name and plugin_key optional, extras dropped
#[must_use]
pub fn plugins(value: &JsonValue) -> bool {
    serde_json::from_value::<Vec<PluginConfig>>(JsonValue::clone(value)).is_ok()
}

// PassThroughGenericEndpoint, one entry of pass_through_endpoints
#[must_use]
pub fn endpoints(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            item.as_object()
                .is_some_and(|entries| check_fields(entries, &ENDPOINT_RULES))
        })
    })
}

fn is_guardrails(value: &JsonValue) -> bool {
    value.as_object().is_some_and(|entries| {
        entries.values().all(|entry| {
            entry.is_null()
                || entry
                    .as_object()
                    .is_some_and(|fields| check_fields(fields, &GUARDRAIL_RULES))
        })
    })
}

// UserHeaderMapping forbids unknown keys
#[must_use]
pub fn header_mappings(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            item.as_object().is_some_and(|entries| {
                only_known(entries, &HEADER_KEYS) && check_fields(entries, &HEADER_RULES)
            })
        })
    })
}

// MCPAdvertisedVersions: the four legacy revisions, at least one
#[must_use]
pub fn advertised_versions(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items
                .iter()
                .all(|item| matches!(item.as_str(), Some(text) if VERSIONS.contains(&text)))
    })
}

// MCPAllowedClient: an alias and a value, each a non-empty string
#[must_use]
pub fn allowed_clients(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|items| {
        items.iter().all(|item| {
            item.as_object()
                .is_some_and(|entries| check_fields(entries, &CLIENT_RULES))
        })
    })
}

// DynamoDBArgs
#[must_use]
pub fn database_args(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    check_fields(entries, &DYNAMO_RULES)
}

// CoordinationRedisParams
#[must_use]
pub fn redis(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    check_fields(entries, &REDIS_RULES)
}

fn is_redis_nodes(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|nodes| {
        nodes.iter().all(|node| {
            node.as_object()
                .is_some_and(|entries| check_fields(entries, &REDIS_NODE_RULES))
        })
    })
}

fn is_sentinel_nodes(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|pairs| {
        pairs.iter().all(|pair| {
            pair.as_array().is_some_and(|parts| {
                parts
                    .iter()
                    .all(|part| part.is_string() || is_integer(part))
            })
        })
    })
}

// ScheduledJobStaggerSettings forbids unknown keys and a window_seconds below zero
#[must_use]
pub fn stagger(value: &JsonValue) -> bool {
    let Some(entries) = value.as_object() else {
        return false;
    };
    only_known(entries, &STAGGER_KEYS) && check_fields(entries, &STAGGER_RULES)
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
    only_known(entries, &SPEND_KEYS) && check_fields(entries, &SPEND_RULES) && threshold && lookback
}

fn non_empty_openai(value: &JsonValue) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty() && items.iter().all(|item| item.as_str() == Some("openai"))
    })
}
