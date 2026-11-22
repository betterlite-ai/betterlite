use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;

mod tests;

pub const POLLING_VIA_CACHE_PATH: &str =
    "litellm_settings.responses.background_mode.polling_via_cache";
pub const NATIVE_BACKGROUND_MODE_PATH: &str =
    "litellm_settings.responses.background_mode.native_background_mode";
pub const TTL_PATH: &str = "litellm_settings.responses.background_mode.ttl";
pub const DEFAULT_POLLING_TTL: i64 = 3600;

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
    raw.convert(environment)
}

impl Raw {
    fn convert(self, lookup: &dyn EnvironmentLookup) -> Result<PollingSettings, ConfigError> {
        let mode = self
            .litellm_settings
            .responses
            .and_then(|responses| responses.background_mode)
            .unwrap_or_default();
        Ok(PollingSettings {
            polling_via_cache: selector(mode.polling_via_cache, lookup)?,
            native_background_mode: native_background_mode(mode.native_background_mode, lookup)?,
            ttl: ttl(mode.ttl, lookup)?,
        })
    }
}

// false, true and numbers are YAML scalars the source accepts; only "all" and provider lists select
fn selector(
    raw: Option<Json>,
    lookup: &dyn EnvironmentLookup,
) -> Result<PollingSelector, ConfigError> {
    match raw {
        None | Some(Json::Null | Json::Bool(false)) => Ok(PollingSelector::Disabled),
        Some(Json::Bool(true) | Json::Number(_)) => Ok(PollingSelector::Unrecognized),
        Some(Json::Text(text)) => {
            let resolved = resolve_string(text, POLLING_VIA_CACHE_PATH, lookup)?;
            if resolved.is_empty() {
                return Ok(PollingSelector::Disabled);
            }
            Ok(match resolved.as_str() {
                "all" => PollingSelector::All,
                _ => PollingSelector::Unrecognized,
            })
        }
        Some(Json::List(items)) => {
            let mut providers = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Json::Text(text) => {
                        providers.push(resolve_string(text, POLLING_VIA_CACHE_PATH, lookup)?);
                    }
                    value @ (Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_)) => {
                        return Err(invalid_scalar(POLLING_VIA_CACHE_PATH, &value));
                    }
                }
            }
            Ok(PollingSelector::Providers(providers))
        }
        Some(value @ Json::Object(_)) => Err(invalid_scalar(POLLING_VIA_CACHE_PATH, &value)),
    }
}

fn native_background_mode(
    raw: Option<Json>,
    lookup: &dyn EnvironmentLookup,
) -> Result<Vec<String>, ConfigError> {
    match raw {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => {
            let mut modes = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Json::Text(text) => {
                        modes.push(resolve_string(text, NATIVE_BACKGROUND_MODE_PATH, lookup)?);
                    }
                    value @ (Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::List(_)
                    | Json::Object(_)) => {
                        return Err(invalid_scalar(NATIVE_BACKGROUND_MODE_PATH, &value));
                    }
                }
            }
            Ok(modes)
        }
        Some(value) => Err(invalid_scalar(NATIVE_BACKGROUND_MODE_PATH, &value)),
    }
}

// the source hands any integer to the cache; only a value that cannot be an integer fails here
fn ttl(raw: Option<RawNumber>, lookup: &dyn EnvironmentLookup) -> Result<i64, ConfigError> {
    match raw {
        None => Ok(DEFAULT_POLLING_TTL),
        Some(RawNumber::Integer(seconds)) => Ok(seconds),
        Some(RawNumber::Text(text)) => match text.strip_prefix("os.environ/") {
            Some(name) => {
                let Some(raw_value) = lookup.lookup(name) else {
                    return Err(ConfigError::MissingEnvironment {
                        variable: name.to_owned(),
                        path: TTL_PATH.to_owned(),
                    });
                };
                Ok(betterlite_env::number::<i64>(name, &raw_value)?.unwrap_or(DEFAULT_POLLING_TTL))
            }
            None => text
                .parse::<i64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: TTL_PATH.to_owned(),
                    source,
                }),
        },
        Some(RawNumber::Float(_)) => Err(ConfigError::InvalidScalar {
            path: TTL_PATH.to_owned(),
            value: false,
        }),
        Some(RawNumber::Boolean(flag)) => Err(ConfigError::InvalidScalar {
            path: TTL_PATH.to_owned(),
            value: flag,
        }),
    }
}

fn invalid_scalar(path: &str, value: &Json) -> ConfigError {
    ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: matches!(value, Json::Bool(true)),
    }
}
