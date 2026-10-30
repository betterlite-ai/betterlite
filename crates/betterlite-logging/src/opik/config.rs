use std::collections::BTreeMap;

use serde_json::{Map, Value};

pub fn get_config_variable(
    key: &str,
    user_value: Option<&str>,
    environment: &BTreeMap<String, String>,
    config_values: &BTreeMap<String, String>,
    default_value: Option<&str>,
) -> Option<String> {
    let value = user_value
        .map(str::to_owned)
        .or_else(|| {
            environment
                .get(&format!("OPIK_{}", key.to_uppercase()))
                .cloned()
        })
        .or_else(|| config_values.get(key).cloned())
        .or_else(|| default_value.map(str::to_owned));
    value
}

pub fn read_config_section(contents: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let mut in_opik = false;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            in_opik = section.trim().eq_ignore_ascii_case("opik");
            continue;
        }
        if !in_opik || line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value
            .split_once('#')
            .map_or(value, |(value, _)| value)
            .trim();
        values.insert(key.trim().to_owned(), unquote(value));
    }
    values
}

pub fn create_usage_object(usage: &Map<String, Value>) -> Map<String, Value> {
    let fields = ["completion_tokens", "prompt_tokens", "total_tokens"]
        .into_iter()
        .filter_map(|key| {
            usage
                .get(key)
                .filter(|value| !value.is_null())
                .map(|value| (key.to_owned(), value.clone()))
        })
        .collect();
    fields
}

fn unquote(value: &str) -> String {
    let unquoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .to_owned();
    unquoted
}
