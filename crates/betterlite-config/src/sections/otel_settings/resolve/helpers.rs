use super::super::{ExclusionInput, RawOtel};
use crate::loader::{ConfigError, EnvironmentLookup};
use serde::de::DeserializeOwned;

pub(in crate::sections::otel_settings) fn csv_values(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn normalized_exclusions(value: Option<ExclusionInput>) -> Vec<String> {
    let items = match value {
        None => return Vec::new(),
        Some(ExclusionInput::Csv(text)) => return csv_values(&text),
        Some(ExclusionInput::List(items)) => items,
        Some(ExclusionInput::Ignored(_)) => {
            tracing::warn!(
                "excluded_services must be a list or comma-separated string; ignoring value"
            );
            return Vec::new();
        }
    };
    items
        .into_iter()
        .filter_map(|item| match item {
            ExclusionInput::Csv(text) => Some(text),
            ExclusionInput::List(_) | ExclusionInput::Ignored(_) => {
                tracing::warn!("excluded_services entries must be strings; ignoring entry");
                None
            }
        })
        .collect()
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
    let environment = names.iter().find_map(|name| {
        lookup
            .lookup(name)
            .or_else(|| {
                lookup
                    .lookup(&name.to_ascii_lowercase())
                    .or_else(|| lookup.lookup(&name.to_ascii_uppercase()))
            })
            .filter(|value| !value.is_empty())
    });
    configured.or_else(|| environment.map(parse))
}

pub(super) fn setting_bool<L>(
    lookup: &L,
    name: &str,
    configured: Option<bool>,
    default: bool,
) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(explicit) = configured {
        return Ok(explicit);
    }
    let Some(value) = lookup.lookup(name) else {
        return Ok(configured.unwrap_or(default));
    };
    betterlite_env::flag(name, &value)
        .map_err(|source| ConfigError::Environment { source })
        .map(|parsed| parsed.unwrap_or(default))
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

pub(super) fn service_identity<L>(raw: &RawOtel, lookup: &L) -> (String, Option<String>)
where
    L: EnvironmentLookup,
{
    let service_name = environment_or(
        lookup,
        &["OTEL_SERVICE_NAME"],
        raw.service_name.clone(),
        |value| value,
    )
    .unwrap_or_else(|| "litellm".to_owned());
    let deployment_environment = environment_or(
        lookup,
        &["OTEL_ENVIRONMENT_NAME"],
        raw.deployment_environment.clone(),
        |value| value,
    );
    (service_name, deployment_environment)
}

pub(super) fn resolve_exclusions<L>(raw: &RawOtel, lookup: &L) -> Vec<String>
where
    L: EnvironmentLookup,
{
    normalized_exclusions(environment_or(
        lookup,
        &["LITELLM_OTEL_EXCLUDED_SERVICES"],
        raw.excluded_services.clone(),
        ExclusionInput::Csv,
    ))
}

pub(super) fn structured_setting<L, T>(
    lookup: &L,
    name: &str,
    configured: Option<T>,
) -> Result<Option<T>, ConfigError>
where
    L: EnvironmentLookup,
    T: DeserializeOwned,
{
    if configured.is_some() {
        return Ok(configured);
    }
    let value = lookup
        .lookup(name)
        .or_else(|| lookup.lookup(&name.to_ascii_uppercase()));
    value
        .filter(|text| !text.is_empty())
        .map(|text| serde_saphyr::from_str(&text).map_err(|source| ConfigError::Yaml { source }))
        .transpose()
}

pub(super) fn shorthand<L>(raw: &RawOtel, lookup: &L) -> [Option<String>; 4]
where
    L: EnvironmentLookup,
{
    [
        (
            ["OTEL_EXPORTER", "OTEL_EXPORTER_OTLP_PROTOCOL"],
            raw.exporter.clone(),
        ),
        (
            ["OTEL_ENDPOINT", "OTEL_EXPORTER_OTLP_ENDPOINT"],
            raw.endpoint.clone(),
        ),
        (
            ["OTEL_TRACES_ENDPOINT", "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"],
            raw.traces_endpoint.clone(),
        ),
        (
            ["OTEL_HEADERS", "OTEL_EXPORTER_OTLP_HEADERS"],
            raw.headers.clone(),
        ),
    ]
    .map(|(names, configured)| environment_or(lookup, &names, configured, |value| value))
}
