use serde_json::{Map, Value};

use super::is_blank;

// Usage.__init__ keeps every extra keyword argument via `setattr`, so unknown usage keys pass through unchanged.
pub(super) fn resolve_usage(raw: Option<Value>) -> Value {
    let fields = match raw.filter(|value| !is_blank(value)) {
        Some(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let mut usage = fields.clone();
    // reasoning_tokens, cost and server_tool_use are named constructor parameters, not passthrough keys.
    for key in ["reasoning_tokens", "cost", "server_tool_use"] {
        usage.remove(key);
    }
    for key in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        let resolved = fields
            .get(key)
            .filter(|value| !is_blank(value))
            .cloned()
            .unwrap_or_else(|| Value::from(0_i64));
        usage.insert(key.to_owned(), resolved);
    }
    for key in ["completion_tokens_details", "prompt_tokens_details"] {
        let resolved = fields
            .get(key)
            .filter(|value| !is_blank(value))
            .cloned()
            .unwrap_or(Value::Null);
        usage.insert(key.to_owned(), resolved);
    }
    fold_reasoning_tokens(&mut usage, &fields);
    if let Some(Value::Object(details)) = usage.get_mut("prompt_tokens_details") {
        mirror_cache_write_tokens(details);
    }
    apply_cache_mappings(&mut usage, &fields);
    // cost and server_tool_use are deleted (left unset) rather than defaulted when the caller omits them.
    if let Some(cost) = fields.get("cost").filter(|value| !value.is_null()).cloned() {
        usage.insert("cost".to_owned(), cost);
    }
    if let Some(server_tool_use) = fields
        .get("server_tool_use")
        .filter(|value| !value.is_null())
        .cloned()
    {
        usage.insert("server_tool_use".to_owned(), server_tool_use);
    }
    Value::Object(usage)
}

// Usage.__init__ folds a truthy reasoning_tokens parameter into completion_tokens_details instead of storing it directly.
fn fold_reasoning_tokens(usage: &mut Map<String, Value>, fields: &Map<String, Value>) {
    let Some(reasoning_tokens) = fields
        .get("reasoning_tokens")
        .filter(|value| !is_blank(value))
        .cloned()
    else {
        return;
    };
    let mut details = match usage.get("completion_tokens_details") {
        Some(Value::Object(existing)) => existing.clone(),
        _ => Map::new(),
    };
    if details.get("reasoning_tokens").is_none_or(Value::is_null) {
        details.insert("reasoning_tokens".to_owned(), reasoning_tokens.clone());
    }
    if details.get("text_tokens").is_none_or(Value::is_null)
        && let Some(completion_tokens) = fields
            .get("completion_tokens")
            .filter(|value| !value.is_null())
            .and_then(Value::as_i64)
    {
        let text_tokens = calculate_text_tokens(completion_tokens, &reasoning_tokens, &details);
        details.insert("text_tokens".to_owned(), Value::from(text_tokens));
    }
    usage.insert(
        "completion_tokens_details".to_owned(),
        Value::Object(details),
    );
}

// Usage.__init__: text_tokens = completion_tokens - reasoning_tokens - image_tokens - audio_tokens, floored at 0.
fn calculate_text_tokens(
    completion_tokens: i64,
    reasoning_tokens: &Value,
    details: &Map<String, Value>,
) -> i64 {
    let reasoning = reasoning_tokens.as_i64().unwrap_or(0_i64);
    let image = details
        .get("image_tokens")
        .filter(|value| !is_blank(value))
        .and_then(Value::as_i64)
        .unwrap_or(0_i64);
    let audio = details
        .get("audio_tokens")
        .filter(|value| !is_blank(value))
        .and_then(Value::as_i64)
        .unwrap_or(0_i64);
    completion_tokens
        .saturating_sub(reasoning)
        .saturating_sub(image)
        .saturating_sub(audio)
        .max(0_i64)
}

// PromptTokensDetailsWrapper keeps cache_write_tokens and cache_creation_tokens mirrored, backfilling from a nested cache_creation_input_tokens key.
fn mirror_cache_write_tokens(details: &mut Map<String, Value>) {
    let from_self = details
        .get("cache_write_tokens")
        .filter(|value| !value.is_null())
        .cloned();
    let from_creation = details
        .get("cache_creation_tokens")
        .filter(|value| !value.is_null())
        .cloned();
    let from_nested = details
        .get("cache_creation_input_tokens")
        .filter(|value| value.is_i64() || value.is_u64())
        .cloned();
    let Some(amount) = from_self.or(from_creation).or(from_nested) else {
        return;
    };
    details.insert("cache_write_tokens".to_owned(), amount.clone());
    details.insert("cache_creation_tokens".to_owned(), amount);
}

// The real Usage object's detail wrappers only expand to their full shape inside original_response.
pub(super) fn original_usage_shape(usage: &Value) -> Value {
    let Value::Object(fields) = usage else {
        return usage.clone();
    };
    let mut expanded = fields.clone();
    expand_usage_details(&mut expanded);
    Value::Object(expanded)
}

// The DEEPSEEK and ANTHROPIC mappings in Usage.__init__: an integer cache-key param also sets prompt_tokens_details.
fn apply_cache_mappings(usage: &mut Map<String, Value>, fields: &Map<String, Value>) {
    let sources = [
        ("prompt_cache_hit_tokens", "cached_tokens"),
        ("cache_read_input_tokens", "cached_tokens"),
        ("cache_creation_input_tokens", "cache_write_tokens"),
    ];
    let amounts: Vec<_> = sources
        .into_iter()
        .filter_map(|(source, target_key)| {
            let amount = fields.get(source)?;
            (amount.is_i64() || amount.is_u64()).then(|| (target_key, amount.clone()))
        })
        .collect();
    if amounts.is_empty() {
        return;
    }
    let mut details = match usage.get("prompt_tokens_details") {
        Some(Value::Object(existing)) => existing.clone(),
        _ => Map::new(),
    };
    for (target_key, amount) in amounts {
        details.insert(target_key.to_owned(), amount.clone());
        // Assigning cache_write_tokens goes through PromptTokensDetailsWrapper.__setattr__, which mirrors it too.
        if target_key == "cache_write_tokens" {
            details.insert("cache_creation_tokens".to_owned(), amount);
        }
    }
    usage.insert("prompt_tokens_details".to_owned(), Value::Object(details));
}

// The wrapper types always serialize their full declared field set, filling an omitted field with null.
const COMPLETION_TOKENS_DETAIL_KEYS: [&str; 7] = [
    "accepted_prediction_tokens",
    "audio_tokens",
    "reasoning_tokens",
    "rejected_prediction_tokens",
    "text_tokens",
    "image_tokens",
    "video_tokens",
];
const PROMPT_TOKENS_DETAIL_KEYS: [&str; 5] = [
    "audio_tokens",
    "cached_tokens",
    "text_tokens",
    "image_tokens",
    "video_tokens",
];
const SERVER_TOOL_USE_KEYS: [&str; 3] = [
    "web_search_requests",
    "tool_search_requests",
    "browser_open_requests",
];

fn expand_details_shape(raw: &Value, always_present: &[&str]) -> Value {
    let Value::Object(fields) = raw else {
        return Value::Null;
    };
    let mut expanded = Map::new();
    for key in always_present {
        expanded.insert((*key).to_owned(), Value::Null);
    }
    for (key, value) in fields {
        expanded.insert(key.clone(), value.clone());
    }
    Value::Object(expanded)
}

// The shared usage object carries the full wrapper shape once a details object is truthy.
fn expand_usage_details(usage: &mut Map<String, Value>) {
    for (key, keys) in [
        (
            "completion_tokens_details",
            COMPLETION_TOKENS_DETAIL_KEYS.as_slice(),
        ),
        (
            "prompt_tokens_details",
            PROMPT_TOKENS_DETAIL_KEYS.as_slice(),
        ),
        ("server_tool_use", SERVER_TOOL_USE_KEYS.as_slice()),
    ] {
        let Some(details) = usage.get(key).filter(|value| !value.is_null()).cloned() else {
            continue;
        };
        usage.insert(key.to_owned(), expand_details_shape(&details, keys));
    }
}
