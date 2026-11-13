use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const KEY_MARKER: &str = "key=";
const API_BASE_MASK: &str = "*****";
const SHORT_VALUE_MASK: &str = "*****";
const MIDDLE_MASK: &str = "****";
const API_BASE_TAIL_CHARS: usize = 4;
const UNMASKED_CHARS: usize = 4;
const EDGE_CHARS: usize = 2;
const NESTING_LIMIT: usize = 20;
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

// LiteLLM slices by code point, so the tail is taken by chars and never by bytes
fn last_chars(text: &str, count: usize) -> String {
    let mut tail: Vec<char> = text.chars().rev().take(count).collect();
    tail.reverse();
    tail.into_iter().collect()
}

#[must_use]
pub fn mask_api_base(api_base: &str) -> String {
    let Some((before_marker, _)) = api_base.split_once(KEY_MARKER) else {
        return api_base.to_owned();
    };
    format!(
        "{before_marker}{KEY_MARKER}{API_BASE_MASK}{}",
        last_chars(api_base, API_BASE_TAIL_CHARS)
    )
}

fn mask_value(value: &Json, depth: usize) -> Json {
    match value {
        Json::Object(_) if depth >= NESTING_LIMIT => value.clone(),
        Json::Object(inner) => Json::Object(mask_level(inner, depth.saturating_add(1))),
        Json::Text(text) if text.chars().count() <= UNMASKED_CHARS => {
            Json::Text(SHORT_VALUE_MASK.to_owned())
        }
        Json::Text(text) => Json::Text(format!(
            "{}{MIDDLE_MASK}{}",
            text.chars().take(EDGE_CHARS).collect::<String>(),
            last_chars(text, EDGE_CHARS)
        )),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => value.clone(),
    }
}

fn mask_level(values: &IndexMap<String, Json>, depth: usize) -> IndexMap<String, Json> {
    let mut masked = IndexMap::new();
    for (name, value) in values {
        let lowered = name.to_lowercase();
        let sensitive = SENSITIVE_KEYWORDS
            .iter()
            .any(|keyword| lowered.contains(keyword));
        let shown = if sensitive {
            mask_value(value, depth)
        } else {
            value.clone()
        };
        masked.insert(name.clone(), shown);
    }
    masked
}

#[must_use]
pub fn masked_headers(
    headers: &IndexMap<String, Json>,
    ignore_sensitive_headers: bool,
) -> IndexMap<String, Json> {
    if ignore_sensitive_headers {
        return headers.clone();
    }
    mask_level(headers, 0)
}
