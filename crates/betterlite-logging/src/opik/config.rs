use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[must_use]
pub fn get_config_variable(
    key: &str,
    user_value: Option<&str>,
    environment: &BTreeMap<String, String>,
    config_values: &BTreeMap<String, String>,
    default_value: Option<&str>,
) -> Option<String> {
    if let Some(value) = user_value {
        return Some(value.to_owned());
    }
    let environment_key = format!("OPIK_{}", key.to_uppercase());
    if let Some(value) = environment.get(&environment_key) {
        return Some(value.clone());
    }
    if let Some(value) = config_values.get(key) {
        return Some(value.clone());
    }
    default_value.map(str::to_owned)
}

#[must_use]
pub fn read_config_section(contents: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let mut in_opik = false;
    for raw_line in contents.lines() {
        let trimmed = raw_line.trim();
        if let Some(section) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_opik = section.trim().eq_ignore_ascii_case("opik");
            continue;
        }
        if !in_opik || trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some((key, raw_value)) = trimmed.split_once('=') else {
            continue;
        };
        let stripped = raw_value
            .split_once('#')
            .map_or(raw_value, |(head, _)| head)
            .trim();
        values.insert(key.trim().to_owned(), unquote(stripped));
    }
    values
}

#[must_use]
pub fn create_usage_object(usage: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let keys = ["completion_tokens", "prompt_tokens", "total_tokens"];
    keys.into_iter()
        .filter_map(|key| {
            usage
                .get(key)
                .filter(|value| !matches!(value, Json::Null))
                .map(|value| (key.to_owned(), value.clone()))
        })
        .collect()
}

fn unquote(value: &str) -> String {
    let double_quoted = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'));
    let single_quoted = value
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''));
    double_quoted.or(single_quoted).unwrap_or(value).to_owned()
}
