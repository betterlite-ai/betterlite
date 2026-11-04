use crate::loader::{ConfigError, EnvironmentLookup};

pub(in crate::sections::otel_settings) fn csv_values(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn normalized_exclusions(value: Option<serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let items = match value {
        serde_json::Value::Null => return Vec::new(),
        serde_json::Value::String(text) => return csv_values(&text),
        serde_json::Value::Array(items) => items,
        other => {
            tracing::warn!(value = %other, "excluded_services must be a list or comma-separated string; ignoring value");
            return Vec::new();
        }
    };
    items.into_iter().filter_map(|item| match item {
        serde_json::Value::String(text) => Some(text),
        other => {
            tracing::warn!(value = %other, "excluded_services entries must be strings; ignoring entry");
            None
        }
    }).collect()
}

pub(super) fn environment_or<L, T, F>(
    lookup: &L,
    names: &[&str],
    configured: Option<T>,
    parse: F,
) -> Option<T>
where
    L: EnvironmentLookup,
    F: FnOnce(String) -> T,
{
    let environment = names.iter().filter_map(|name| lookup.lookup(name)).next();
    match environment {
        Some(value) => Some(parse(value)),
        None => configured,
    }
}

pub(super) fn setting_bool<L: EnvironmentLookup>(
    lookup: &L,
    name: &str,
    configured: Option<bool>,
    default: bool,
) -> Result<bool, ConfigError> {
    let Some(value) = lookup.lookup(name) else {
        return Ok(configured.unwrap_or(default));
    };
    betterlite_env::flag(name, &value)
        .map_err(|source| ConfigError::Environment { source })
        .map(|value| value.unwrap_or(default))
}

pub(super) const DEFAULT_BAGGAGE_PROMOTED_KEYS: &[&str] = &[
    "litellm.team.id",
    "litellm.team.alias",
    "litellm.team.metadata",
    "litellm.api_key.hash",
    "gen_ai.request.model",
    "litellm.provider.model",
];

pub(super) const DEFAULT_BAGGAGE_METADATA_KEYS: &[&str] = &[
    "user_api_key_org_id",
    "user_api_key_user_id",
    "user_api_key_alias",
    "user_api_key_end_user_id",
    "requester_ip_address",
];

pub(super) fn default_baggage_keys(keys: &[&str]) -> Vec<String> {
    let mut output = Vec::with_capacity(keys.len());
    for key in keys {
        output.push((*key).to_owned());
    }
    output
}
