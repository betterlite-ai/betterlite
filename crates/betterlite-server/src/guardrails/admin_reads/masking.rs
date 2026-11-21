use betterlite_pyhost::json_values::{JsonObject, JsonValue};

// LiteLLM's _get_masked_values with unmasked_length 4 and number_of_asterisks 4
const SENSITIVE_KEYWORDS: [&str; 8] = [
    "authorization",
    "token",
    "key",
    "secret",
    "vertex_credentials",
    "credentials",
    "password",
    "passwd",
];
const UNMASKED_LENGTH: usize = 4;
// LiteLLM halves unmasked_length, so four characters show two on each side
const VISIBLE_LENGTH: usize = 2;
const ASTERISKS: &str = "****";
const SHORT_MASK: &str = "*****";
const MAX_DEPTH: usize = 20;

// B3686 calls this on its parsed map, and the pyhost aliases name serde_json's Map and Value without a lint exception
#[must_use]
// ast-grep-ignore: no-one-line-functions
pub(crate) fn mask_sensitive_values(values: &JsonObject) -> JsonObject {
    mask_object(values, 0)
}

fn mask_object(values: &JsonObject, depth: usize) -> JsonObject {
    let mut masked = JsonObject::with_capacity(values.len());
    for (key, value) in values {
        let masked_value = if is_sensitive(key) {
            mask_value(value, depth)
        } else {
            value.clone()
        };
        masked.insert(key.clone(), masked_value);
    }
    masked
}

fn is_sensitive(key: &str) -> bool {
    let lowered = key.to_lowercase();
    SENSITIVE_KEYWORDS
        .iter()
        .any(|keyword| lowered.contains(keyword))
}

// the depth of a dict is checked where LiteLLM enters it, so the parent's depth decides
fn mask_value(value: &JsonValue, depth: usize) -> JsonValue {
    match value {
        JsonValue::Object(inner) => {
            if depth >= MAX_DEPTH {
                return value.clone();
            }
            JsonValue::Object(mask_object(inner, depth.saturating_add(1)))
        }
        JsonValue::String(text) => JsonValue::String(masked_text(text)),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            value.clone()
        }
    }
}

// Python counts code points, so a multi-byte character is one position
fn masked_text(text: &str) -> String {
    let length = text.chars().count();
    if length <= UNMASKED_LENGTH {
        return SHORT_MASK.to_owned();
    }
    let head: String = text.chars().take(VISIBLE_LENGTH).collect();
    let mut tail: Vec<char> = text.chars().rev().take(VISIBLE_LENGTH).collect();
    tail.reverse();
    let mut masked = String::with_capacity(
        VISIBLE_LENGTH
            .saturating_mul(2)
            .saturating_add(ASTERISKS.len()),
    );
    masked.push_str(&head);
    masked.push_str(ASTERISKS);
    masked.extend(tail);
    masked
}
