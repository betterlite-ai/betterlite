use betterlite_models::decisions::{Extra, Json};
use serde_json::Number;

enum Declared {
    Count,
    Seconds,
    Nested(&'static [&'static str]),
    Open,
}

// The fields PromptTokensDetailsWrapper declares; any other key is kept as an extra
fn declared(name: &str) -> Declared {
    match name {
        "audio_tokens"
        | "cached_tokens"
        | "text_tokens"
        | "image_tokens"
        | "video_tokens"
        | "web_search_requests"
        | "google_maps_grounding_requests"
        | "tool_use_tokens"
        | "character_count"
        | "image_count"
        | "query_count"
        | "cache_write_tokens"
        | "cache_creation_tokens" => Declared::Count,
        "video_length_seconds" | "audio_length_seconds" => Declared::Seconds,
        "cache_creation_token_details" => {
            Declared::Nested(&["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"])
        }
        "cached_tokens_details" => {
            Declared::Nested(&["text_tokens", "audio_tokens", "image_tokens"])
        }
        _ => Declared::Open,
    }
}

// PromptTokensDetailsWrapper(**fields): None when a declared field holds the wrong type, nulls are dropped, and the cache write and creation counts mirror
#[must_use]
pub fn build_prompt_tokens_details(fields: Extra) -> Option<Extra> {
    let mut details = Extra::new();
    for (name, value) in fields {
        if matches!(value, Json::Null) {
            continue;
        }
        let checked = match declared(&name) {
            Declared::Count => whole_count(&value)?,
            Declared::Seconds => seconds(&value)?,
            Declared::Nested(keys) => nested_counts(&value, keys)?,
            Declared::Open => value,
        };
        details.insert(name, checked);
    }
    Some(mirror_cache_tokens(details))
}

fn whole_count(value: &Json) -> Option<Json> {
    let Json::Number(number) = value else {
        return None;
    };
    if number.as_i128().is_some() {
        return Some(value.clone());
    }
    let float = number.as_f64()?;
    if !float.is_finite() || float.trunc().to_bits() != float.to_bits() {
        return None;
    }
    let whole = float.to_string().parse::<i128>().ok()?;
    Number::from_i128(whole).map(Json::Number)
}

fn seconds(value: &Json) -> Option<Json> {
    let Json::Number(number) = value else {
        return None;
    };
    Number::from_f64(number.as_f64()?).map(Json::Number)
}

// A nested pydantic model keeps its declared keys and ignores the rest
fn nested_counts(value: &Json, keys: &[&str]) -> Option<Json> {
    let Json::Object(members) = value else {
        return None;
    };
    let mut kept = Extra::new();
    for (name, member) in members {
        if keys.contains(&name.as_str()) && !matches!(member, Json::Null) {
            kept.insert(name.clone(), whole_count(member)?);
        }
    }
    Some(Json::Object(kept))
}

// cache_write_tokens wins, then cache_creation_tokens, then an integer cache_creation_input_tokens extra
fn mirror_cache_tokens(mut details: Extra) -> Extra {
    let nested = details
        .get("cache_creation_input_tokens")
        .filter(|value| matches!(value, Json::Number(number) if number.as_i128().is_some()))
        .cloned();
    let mirrored = ["cache_write_tokens", "cache_creation_tokens"]
        .iter()
        .find_map(|name| details.get(*name).cloned())
        .or(nested);
    if let Some(count) = mirrored {
        details.insert("cache_write_tokens".to_owned(), count.clone());
        details.insert("cache_creation_tokens".to_owned(), count);
    }
    details
}
