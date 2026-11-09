use super::compat_callbacks::{CallbackConfig, get_callback_config, substitute_env_variables};
use betterlite_config::loader::EnvironmentLookup;
use indexmap::IndexMap;
use std::str::FromStr as _;
use std::time::Duration;
mod tests;
pub const ENDPOINT_VARIABLE: &str = "GENERIC_LOGGER_ENDPOINT";
pub const HEADERS_VARIABLE: &str = "GENERIC_LOGGER_HEADERS";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum LogFormat {
    #[default]
    JsonArray,
    Ndjson,
    Single,
}
#[derive(Clone, Debug, Default)]
pub struct GenericApiArgs {
    pub callback_name: Option<String>,
    pub endpoint: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub headers: Option<IndexMap<String, String>>,
    pub log_format: Option<String>,
    pub max_retries: Option<i64>,
    pub retry_delay: Option<f64>,
    pub timeout: Option<Duration>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct GenericApiSettings {
    pub callback_name: Option<String>,
    pub endpoint: String,
    pub event_types: Option<Vec<String>>,
    pub headers: IndexMap<String, String>,
    pub log_format: LogFormat,
    pub max_retries: u32,
    pub retry_delay: f64,
    pub timeout: Option<Duration>,
}
#[derive(Debug, thiserror::Error)]
pub enum GenericApiSettingsError {
    #[error(
        "endpoint not set for GenericAPILogger, GENERIC_LOGGER_ENDPOINT not found in environment variables"
    )]
    MissingEndpoint,
    #[error("Invalid log_format: {0}. Must be one of: 'json_array', 'ndjson', 'single'")]
    InvalidLogFormat(String),
}
fn named_config(name: &str) -> Option<CallbackConfig> {
    let config = get_callback_config(name);
    if config.is_none() {
        tracing::warn!("callback_name '{name}' not found in generic_api_compatible_callbacks.json");
    }
    config
}
fn add_config_headers(
    given: &mut IndexMap<String, String>,
    config: &CallbackConfig,
    environment: &dyn EnvironmentLookup,
) {
    let Some(headers) = &config.headers else {
        return;
    };
    for (key, value) in headers {
        if !given.contains_key(key) {
            given.insert(key.clone(), substitute_env_variables(value, environment));
        }
    }
}
fn merged_headers(
    given: IndexMap<String, String>,
    global_headers: &IndexMap<String, String>,
    environment: &dyn EnvironmentLookup,
) -> IndexMap<String, String> {
    let mut headers = IndexMap::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    let env_headers = environment.lookup(HEADERS_VARIABLE).unwrap_or_default();
    for item in env_headers.split(',').filter(|item| !item.is_empty()) {
        if let Some((key, value)) = item.split_once('=') {
            headers.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    headers.extend(global_headers.clone());
    headers.extend(given);
    headers
}
fn resolved_format(log_format: Option<String>) -> Result<LogFormat, GenericApiSettingsError> {
    log_format.map_or(Ok(LogFormat::JsonArray), |text| {
        LogFormat::from_str(&text)
            .map_err(|_invalid| GenericApiSettingsError::InvalidLogFormat(text))
    })
}
pub fn resolve(
    args: GenericApiArgs,
    environment: &dyn EnvironmentLookup,
    global_headers: &IndexMap<String, String>,
) -> Result<GenericApiSettings, GenericApiSettingsError> {
    let named = args.callback_name.as_deref().and_then(named_config);
    let mut endpoint = args.endpoint;
    let mut headers = args.headers.unwrap_or_default();
    let mut event_types = args.event_types;
    let mut log_format = args.log_format;
    if let Some(configured) = &named {
        if endpoint.as_deref().is_none_or(str::is_empty) {
            endpoint = configured
                .endpoint
                .as_deref()
                .map(|text| substitute_env_variables(text, environment));
        }
        add_config_headers(&mut headers, configured, environment);
        event_types = event_types.or_else(|| configured.event_types.clone());
        log_format = log_format.or_else(|| configured.log_format.clone());
    }
    let resolved_endpoint = endpoint
        .filter(|text| !text.is_empty())
        .or_else(|| {
            environment
                .lookup(ENDPOINT_VARIABLE)
                .filter(|text| !text.is_empty())
        })
        .ok_or(GenericApiSettingsError::MissingEndpoint)?;
    Ok(GenericApiSettings {
        callback_name: args.callback_name,
        endpoint: resolved_endpoint,
        event_types,
        headers: merged_headers(headers, global_headers, environment),
        log_format: resolved_format(log_format)?,
        max_retries: u32::try_from(args.max_retries.unwrap_or(0).max(0)).unwrap_or(u32::MAX),
        retry_delay: args.retry_delay.unwrap_or(0.0_f64).max(0.0_f64),
        timeout: args.timeout,
    })
}
