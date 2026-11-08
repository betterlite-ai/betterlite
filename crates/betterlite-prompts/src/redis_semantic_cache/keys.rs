use sha2::{Digest as _, Sha256};

use super::CACHE_KEY_FIELD_NAME;
use super::ports::CacheEntryFields;

// RedisVL's TAG_ESCAPED_CHARS, with `|` because an unescaped union splits a single tag value.
const TAG_ESCAPED_CHARS: [char; 31] = [
    ',', '.', '<', '>', '{', '}', '[', ']', '\\', '"', '\'', ':', ';', '!', '@', '#', '$', '%',
    '^', '&', '*', '(', ')', '-', '+', '=', '~', '|', '/', ' ', '?',
];

pub(crate) fn entry_id(prompt: &str, cache_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prompt.as_bytes());
    hasher.update(CACHE_KEY_FIELD_NAME.as_bytes());
    hasher.update(cache_key.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn escape_tag_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if TAG_ESCAPED_CHARS.contains(&character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn entry_fields(
    prompt: &str,
    response: String,
    vector: &[f32],
    cache_key: &str,
    timestamp: f64,
) -> CacheEntryFields {
    CacheEntryFields {
        entry_id: entry_id(prompt, cache_key),
        prompt: prompt.to_owned(),
        response,
        prompt_vector: vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect(),
        inserted_at: timestamp,
        updated_at: timestamp,
        litellm_cache_key: cache_key.to_owned(),
    }
}
