use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

fn patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?m)^#\s*(\S+\.\w+)",
            r"(?m)^//\s*(\S+\.\w+)",
            r"(?m)^File:\s*(\S+)",
            r"(?m)^---\s*(\S+\.\w+)",
            r"`(\S+\.\w{1,5})`",
        ]
        .into_iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .collect()
    })
}

pub fn extract_key(message: &Value, fallback_index: usize, used_keys: &mut Vec<String>) -> String {
    let content = message
        .get("content")
        .map(content_to_string)
        .unwrap_or_default();
    let end = content
        .char_indices()
        .nth(2000)
        .map_or(content.len(), |(index, _)| index);
    let sample = &content[..end];
    let mut key = patterns()
        .iter()
        .find_map(|pattern| pattern.captures(sample).and_then(|capture| capture.get(1)))
        .map_or_else(
            || format!("message_{fallback_index}"),
            |match_| {
                match_
                    .as_str()
                    .rsplit('/')
                    .next()
                    .map_or_else(String::new, str::to_owned)
            },
        );
    let base = key.clone();
    let mut suffix = 2_usize;
    while used_keys.contains(&key) {
        key = format!("{base}_{suffix}");
        suffix = suffix.saturating_add(1);
    }
    used_keys.push(key.clone());
    key
}

pub fn content_to_string(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| part.as_str().unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

mod tests;
