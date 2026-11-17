use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::fields::usage::Usage;

// LiteLLM's backfill_missing_cache_usage_fields, without overwriting a count already set.
pub fn backfill_cache_usage(usage: &mut Usage) {
    let details = usage.prompt_tokens_details.as_ref();
    if usage
        .extras
        .get("cache_read_input_tokens")
        .is_none_or(|value| matches!(value, Json::Null))
    {
        let cached = cached_tokens(details);
        usage.extras.insert(
            "cache_read_input_tokens".to_owned(),
            Json::Number(Number::from(cached)),
        );
    }
    if usage
        .extras
        .get("cache_creation_input_tokens")
        .is_none_or(|value| matches!(value, Json::Null))
    {
        let creation = creation_tokens(details);
        usage.extras.insert(
            "cache_creation_input_tokens".to_owned(),
            Json::Number(Number::from(creation)),
        );
    }
    if usage.prompt_tokens_details.is_none() {
        let mut defaults = IndexMap::new();
        defaults.insert(
            "cached_tokens".to_owned(),
            Json::Number(Number::from(0_i64)),
        );
        usage.prompt_tokens_details = Some(defaults);
    }
}

fn cached_tokens(recorded: Option<&IndexMap<String, Json>>) -> i64 {
    recorded
        .and_then(|counts| nonzero(counts.get("cached_tokens")))
        .unwrap_or(0)
}

// The wrapper mirrors cache_write_tokens and cache_creation_tokens, first present wins.
fn creation_tokens(recorded: Option<&IndexMap<String, Json>>) -> i64 {
    let Some(details) = recorded else {
        return 0;
    };
    for key in [
        "cache_write_tokens",
        "cache_creation_tokens",
        "cache_creation_input_tokens",
    ] {
        if let Some(value) = details
            .get(key)
            .filter(|value| !matches!(value, Json::Null))
        {
            return count_value(value);
        }
    }
    0
}

fn nonzero(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64().filter(|count| *count != 0),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn count_value(value: &Json) -> i64 {
    match value {
        Json::Number(number) => number.as_i64().unwrap_or(0),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => 0,
    }
}
