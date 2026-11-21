use betterlite_models::baseline_history::cache_plan::{CountedBreakpoint, CountedPromptCachePlan};
use betterlite_models::baseline_history::usage::{
    CacheCreationTokenDetails, PromptTokensDetails, Usage,
};
use betterlite_models::decisions::Json;

use super::super::Canonical;

const PROMPT_ALWAYS: [&str; 5] = [
    "audio_tokens",
    "cached_tokens",
    "text_tokens",
    "image_tokens",
    "video_tokens",
];
const PROMPT_DROPPED_WHEN_NULL: [&str; 9] = [
    "character_count",
    "image_count",
    "video_length_seconds",
    "audio_length_seconds",
    "query_count",
    "web_search_requests",
    "google_maps_grounding_requests",
    "tool_use_tokens",
    "cached_tokens_details",
];
const COMPLETION_ALWAYS: [&str; 7] = [
    "accepted_prediction_tokens",
    "audio_tokens",
    "image_tokens",
    "reasoning_tokens",
    "rejected_prediction_tokens",
    "text_tokens",
    "video_tokens",
];

pub(super) fn usage_projection(value: &Usage) -> Canonical {
    let mut root = Canonical::object();
    root.insert("prompt_tokens", Canonical::Integer(value.prompt_tokens));
    root.insert(
        "completion_tokens",
        Canonical::Integer(value.completion_tokens),
    );
    root.insert("total_tokens", Canonical::Integer(value.total_tokens));
    root.insert(
        "prompt_tokens_details",
        value
            .prompt_tokens_details
            .as_ref()
            .map_or(Canonical::Null, prompt_details),
    );
    root.insert(
        "completion_tokens_details",
        completion_details(value.extra.get("completion_tokens_details")),
    );
    if let Some(cached) = value.cache_read_input_tokens {
        root.insert("cache_read_input_tokens", Canonical::Integer(cached));
    }
    if let Some(creation) = value.cache_creation_input_tokens {
        root.insert("cache_creation_input_tokens", Canonical::Integer(creation));
    }
    for (key, child) in &value.extra {
        if key != "completion_tokens_details" {
            root.insert(key, Canonical::from(child));
        }
    }
    root
}

fn prompt_details(value: &PromptTokensDetails) -> Canonical {
    let mut root = Canonical::object();
    root.insert("audio_tokens", extra_or_null(value, "audio_tokens"));
    root.insert("cached_tokens", option_integer(value.cached_tokens));
    root.insert("text_tokens", option_integer(value.text_tokens));
    root.insert("image_tokens", extra_or_null(value, "image_tokens"));
    root.insert("video_tokens", extra_or_null(value, "video_tokens"));
    if let Some(creation) = value.cache_creation_tokens {
        root.insert("cache_creation_tokens", Canonical::Integer(creation));
    }
    if let Some(write) = value.cache_write_tokens {
        root.insert("cache_write_tokens", Canonical::Integer(write));
    }
    if let Some(details) = &value.cache_creation_token_details {
        root.insert(
            "cache_creation_token_details",
            cache_creation_details(details),
        );
    }
    for (key, child) in &value.extra {
        let always = PROMPT_ALWAYS.contains(&key.as_str());
        let dropped =
            PROMPT_DROPPED_WHEN_NULL.contains(&key.as_str()) && matches!(child, Json::Null);
        if !always && !dropped {
            root.insert(key, Canonical::from(child));
        }
    }
    root
}

fn extra_or_null(value: &PromptTokensDetails, key: &str) -> Canonical {
    value
        .extra
        .get(key)
        .map_or(Canonical::Null, Canonical::from)
}

fn cache_creation_details(value: &CacheCreationTokenDetails) -> Canonical {
    let mut root = Canonical::object();
    root.insert(
        "ephemeral_5m_input_tokens",
        option_integer(value.ephemeral_5m_input_tokens),
    );
    root.insert(
        "ephemeral_1h_input_tokens",
        option_integer(value.ephemeral_1h_input_tokens),
    );
    root
}

fn completion_details(value: Option<&Json>) -> Canonical {
    let Some(Json::Object(fields)) = value else {
        return Canonical::Null;
    };
    let mut root = Canonical::object();
    for key in COMPLETION_ALWAYS {
        root.insert(
            key,
            fields.get(key).map_or(Canonical::Null, Canonical::from),
        );
    }
    for (key, child) in fields {
        if !COMPLETION_ALWAYS.contains(&key.as_str()) {
            root.insert(key, Canonical::from(child));
        }
    }
    root
}

pub(super) fn plan_projection(value: &CountedPromptCachePlan) -> Canonical {
    let mut root = Canonical::object();
    root.insert("total_tokens", Canonical::Integer(value.total_tokens));
    root.insert(
        "breakpoints",
        Canonical::List(value.breakpoints.iter().map(breakpoint).collect()),
    );
    root
}

fn breakpoint(value: &CountedBreakpoint) -> Canonical {
    let mut root = Canonical::object();
    root.insert("fingerprint", Canonical::Text(value.fingerprint.clone()));
    root.insert("ttl_seconds", Canonical::Integer(value.ttl_seconds));
    root.insert("prefix_tokens", Canonical::Integer(value.prefix_tokens));
    root.insert("lookback_fingerprints", texts(&value.lookback_fingerprints));
    root.insert(
        "content_fingerprint",
        Canonical::Text(value.content_fingerprint.clone()),
    );
    root.insert(
        "lookback_content_fingerprints",
        texts(&value.lookback_content_fingerprints),
    );
    root
}

fn texts(values: &[String]) -> Canonical {
    Canonical::List(
        values
            .iter()
            .map(|text| Canonical::Text(text.clone()))
            .collect(),
    )
}

fn option_integer(value: Option<i64>) -> Canonical {
    value.map_or(Canonical::Null, Canonical::Integer)
}
