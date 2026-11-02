use serde_json::{Map, Value};

use super::is_blank;

// Usage.__init__ keeps every extra keyword argument via `setattr`, so unknown usage keys pass through unchanged.
pub(super) fn resolve_usage(raw: Option<Value>) -> Value {
    let fields = match raw.filter(|value| !is_blank(value)) {
        Some(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let mut usage = fields.clone();
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
    apply_cache_mappings(&mut usage, &fields);
    Value::Object(usage)
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
        details.insert(target_key.to_owned(), amount);
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
    ] {
        let Some(details) = usage.get(key).filter(|value| !value.is_null()).cloned() else {
            continue;
        };
        usage.insert(key.to_owned(), expand_details_shape(&details, keys));
    }
}
