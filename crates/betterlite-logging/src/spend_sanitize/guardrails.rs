use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::REDACTED_BY_LITELM;
use super::SpendSanitizePolicy;
use super::value::{SpendMapping, SpendValue, to_json};

const PROMPT_CARRYING_FIELDS: [&str; 4] = [
    "guardrail_request",
    "guardrail_response",
    "match_details",
    "classification",
];
const NUMERIC_STAT_FIELDS: [&str; 4] = [
    "tokens_before",
    "tokens_after",
    "tokens_saved",
    "compression_ratio",
];

pub fn sanitize_guardrail_information(
    information: &SpendValue,
    policy: &SpendSanitizePolicy,
) -> Json {
    if policy.store_bodies {
        return to_json(information);
    }
    match information {
        SpendValue::Mapping(entry) => Json::List(vec![redact_entry(entry)]),
        SpendValue::List(list) => Json::List(
            list.items
                .iter()
                .filter_map(as_mapping)
                .map(redact_entry)
                .collect(),
        ),
        // LiteLLM returns None for an absent value and an empty list for a value outside the type
        SpendValue::Null => Json::Null,
        SpendValue::Bool(_)
        | SpendValue::Number(_)
        | SpendValue::Text(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => Json::List(Vec::new()),
    }
}

const fn as_mapping(value: &SpendValue) -> Option<&SpendMapping> {
    match value {
        SpendValue::Mapping(mapping) => Some(mapping),
        SpendValue::Null
        | SpendValue::Bool(_)
        | SpendValue::Number(_)
        | SpendValue::Text(_)
        | SpendValue::List(_)
        | SpendValue::Model(_)
        | SpendValue::Object(_) => None,
    }
}

fn redact_entry(entry: &SpendMapping) -> Json {
    let preserved = numeric_stats(entry.entries.get("guardrail_response"));
    let mut redacted = IndexMap::new();
    for (key, value) in &entry.entries {
        let item = if PROMPT_CARRYING_FIELDS.contains(&key.as_str()) {
            Json::Text(REDACTED_BY_LITELM.to_owned())
        } else {
            to_json(value)
        };
        redacted.insert(key.clone(), item);
    }
    if let Some(stats) = preserved {
        redacted.insert("guardrail_response".to_owned(), stats);
    }
    Json::Object(redacted)
}

// Spend aggregation reads these compression counters, and a number cannot carry prompt text
fn numeric_stats(response: Option<&SpendValue>) -> Option<Json> {
    let SpendValue::Mapping(mapping) = response? else {
        return None;
    };
    let stats = mapping
        .entries
        .iter()
        .filter(|(key, value)| {
            NUMERIC_STAT_FIELDS.contains(&key.as_str()) && matches!(value, SpendValue::Number(_))
        })
        .map(|(key, value)| (key.clone(), to_json(value)))
        .collect::<IndexMap<_, _>>();
    if stats.is_empty() {
        None
    } else {
        Some(Json::Object(stats))
    }
}
