use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};

mod tests;

pub const POLLING_VIA_CACHE_PATH: &str =
    "litellm_settings.responses.background_mode.polling_via_cache";
pub const NATIVE_BACKGROUND_MODE_PATH: &str =
    "litellm_settings.responses.background_mode.native_background_mode";
pub const TTL_PATH: &str = "litellm_settings.responses.background_mode.ttl";
pub const DEFAULT_POLLING_TTL: i64 = 3600;

type SelectorResult = Result<PollingSelector, ConfigError>;
type ProviderNames = Result<Vec<String>, ConfigError>;
type TtlSeconds = Result<i64, ConfigError>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PollingSelector {
    #[default]
    Disabled,
    All,
    Providers(Vec<String>),
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingSettings {
    pub polling_via_cache: PollingSelector,
    pub native_background_mode: Vec<String>,
    pub ttl: i64,
}

impl Default for PollingSettings {
    fn default() -> Self {
        Self {
            polling_via_cache: PollingSelector::Disabled,
            native_background_mode: Vec::new(),
            ttl: DEFAULT_POLLING_TTL,
        }
    }
}

// the proxy's startup branch reads exactly these three keys through the loader's own YAML decoding
#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    responses: Option<RawResponses>,
}

#[derive(Debug, Default, Deserialize)]
struct RawResponses {
    background_mode: Option<RawBackgroundMode>,
}

#[derive(Debug, Default, Deserialize)]
struct RawBackgroundMode {
    polling_via_cache: Option<Json>,
    native_background_mode: Option<Json>,
    ttl: Option<RawNumber>,
}

pub fn loads_polling_settings(
    source: &str,
    environment: &dyn EnvironmentLookup,
) -> Result<PollingSettings, ConfigError> {
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    let mode = raw
        .litellm_settings
        .responses
        .and_then(|responses| responses.background_mode)
        .unwrap_or_default();
    Ok(PollingSettings {
        polling_via_cache: selector(mode.polling_via_cache, environment)?,
        native_background_mode: native_background_mode(mode.native_background_mode, environment)?,
        ttl: ttl(mode.ttl, environment)?,
    })
}

// false, true and numbers are YAML scalars the source accepts; only "all" and provider lists select
fn selector(raw: Option<Json>, lookup: &dyn EnvironmentLookup) -> SelectorResult {
    match raw {
        None | Some(Json::Null | Json::Bool(false)) => Ok(PollingSelector::Disabled),
        Some(Json::Bool(true) | Json::Number(_)) => Ok(PollingSelector::Unrecognized),
        Some(Json::Text(text)) => selector_text(text, lookup),
        Some(Json::List(items)) => {
            resolve_list(&items, POLLING_VIA_CACHE_PATH, lookup).map(PollingSelector::Providers)
        }
        Some(value @ Json::Object(_)) => Err(invalid_scalar(POLLING_VIA_CACHE_PATH, &value)),
    }
}

fn selector_text(text: String, lookup: &dyn EnvironmentLookup) -> SelectorResult {
    let resolved = resolve_string(text, POLLING_VIA_CACHE_PATH, lookup)?;
    Ok(match resolved.as_str() {
        "" => PollingSelector::Disabled,
        "all" => PollingSelector::All,
        _ => PollingSelector::Unrecognized,
    })
}

fn native_background_mode(raw: Option<Json>, lookup: &dyn EnvironmentLookup) -> ProviderNames {
    match raw {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => resolve_list(&items, NATIVE_BACKGROUND_MODE_PATH, lookup),
        Some(value) => Err(invalid_scalar(NATIVE_BACKGROUND_MODE_PATH, &value)),
    }
}

fn resolve_list(items: &[Json], path: &str, lookup: &dyn EnvironmentLookup) -> ProviderNames {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(text) = item else {
            return Err(invalid_scalar(path, item));
        };
        values.push(resolve_string(text.clone(), path, lookup)?);
    }
    Ok(values)
}

// the source hands any integer to the cache; a fractional ttl is an invalid scalar, which
// resolve_number would report as an invalid integer, so only that arm stays local
fn ttl(raw: Option<RawNumber>, lookup: &dyn EnvironmentLookup) -> TtlSeconds {
    if matches!(raw, Some(RawNumber::Float(_))) {
        return Err(invalid_scalar(TTL_PATH, &Json::Null));
    }
    Ok(resolve_number(raw, TTL_PATH, lookup, parse_ttl)?.unwrap_or(DEFAULT_POLLING_TTL))
}

fn parse_ttl(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

fn invalid_scalar(path: &str, value: &Json) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: matches!(value, Json::Bool(true)),
    }
}
