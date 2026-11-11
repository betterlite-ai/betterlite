use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

type UsageMapping = IndexMap<String, Json>;

const PRESENCE_FIELDS: [&str; 4] = [
    "prompt_tokens",
    "completion_tokens",
    "input_tokens",
    "output_tokens",
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTokens {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
}

impl UsageTokens {
    // LiteLLM's _usage_is_present plus _get_usage_tokens: a bare total_tokens has no usable breakdown
    #[must_use]
    pub fn from_usage(usage: &Json) -> Option<Self> {
        let Json::Object(mapping) = usage else {
            return None;
        };
        if !PRESENCE_FIELDS
            .iter()
            .any(|field| mapping.contains_key(*field))
        {
            return None;
        }
        Some(Self {
            prompt_tokens: first_nonzero(mapping, &["prompt_tokens", "input_tokens"]),
            completion_tokens: first_nonzero(mapping, &["completion_tokens", "output_tokens"]),
            cached_tokens: cached_usage_tokens(mapping),
        })
    }

    // the standard_logging_object copy: prompt and completion tokens plus metadata.cache_read_input_tokens
    #[must_use]
    pub fn from_standard_logging(logging: &Json) -> Option<Self> {
        let Json::Object(mapping) = logging else {
            return None;
        };
        let cached_tokens = match mapping.get("metadata") {
            Some(Json::Object(metadata)) => first_nonzero(metadata, &["cache_read_input_tokens"]),
            Some(_) | None => 0,
        };
        Some(Self {
            prompt_tokens: first_nonzero(mapping, &["prompt_tokens"]),
            completion_tokens: first_nonzero(mapping, &["completion_tokens"]),
            cached_tokens,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileUsage {
    pub billable_input: i64,
    pub completion_tokens: i64,
    pub usage_resolved: bool,
}

// None stands for the pinned _usage_is_present false, which a usage carrying only total_tokens also is
#[must_use]
pub fn resolve_reconcile_usage(
    usage: Option<UsageTokens>,
    standard_logging: Option<UsageTokens>,
) -> ReconcileUsage {
    if let Some(resolved_usage) = usage {
        return ReconcileUsage {
            billable_input: resolved_usage
                .prompt_tokens
                .saturating_sub(resolved_usage.cached_tokens)
                .max(0),
            completion_tokens: resolved_usage.completion_tokens,
            usage_resolved: true,
        };
    }
    if let Some(stored) = standard_logging
        && (stored.prompt_tokens > 0 || stored.completion_tokens > 0)
    {
        return ReconcileUsage {
            billable_input: stored
                .prompt_tokens
                .saturating_sub(stored.cached_tokens)
                .max(0),
            completion_tokens: stored.completion_tokens,
            usage_resolved: true,
        };
    }
    ReconcileUsage::default()
}

// LiteLLM takes the first non-zero alias, so an explicit prompt_tokens of 0 still falls back to input_tokens
fn first_nonzero(usage: &UsageMapping, fields: &[&str]) -> i64 {
    for field in fields {
        if let Some(value) = number_value(usage.get(*field))
            && value != 0
        {
            return value;
        }
    }
    0
}

// prompt_tokens_details.cached_tokens wins over the top-level cache_read_input_tokens
fn cached_usage_tokens(usage: &UsageMapping) -> i64 {
    let details = match usage.get("prompt_tokens_details") {
        Some(Json::Object(details)) => first_nonzero(details, &["cached_tokens"]),
        Some(_) | None => 0,
    };
    if details != 0 {
        return details;
    }
    first_nonzero(usage, &["cache_read_input_tokens"])
}

fn number_value(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Number(number)) => number.as_i64(),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) | Json::Null)
        | None => None,
    }
}
