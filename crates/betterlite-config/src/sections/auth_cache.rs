use std::time::{Duration, TryFromFloatSecsError};

use serde::Deserialize;
use serde::de::Error as DeserializeError;
use serde::de::IgnoredAny;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const FALLBACK_VARIABLE: &str = "DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL";
const TTL_PATH: &str = "general_settings.user_api_key_cache_ttl";
const MAX_SIZE_PATH: &str = "general_settings.user_api_key_cache_max_size";
// litellm/proxy_server.py's UserAPIKeyCacheTTLEnum.in_memory_cache_ttl
const DEFAULT_GATEWAY_MEMORY_TTL: f64 = 60.0;
// litellm/constants.py's DEFAULT_MANAGEMENT_OBJECT_IN_MEMORY_CACHE_TTL default
const DEFAULT_MANAGEMENT_TTL: f64 = 60.0;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    user_api_key_cache_ttl: Option<RawScalar>,
    user_api_key_cache_max_size: Option<RawScalar>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawScalar {
    Integer(i64),
    Float(f64),
    Text(String),
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub management_object_ttl: f64,
    pub gateway_memory_ttl: f64,
    pub gateway_redis_ttl: Option<f64>,
    pub user_api_key_cache_max_size: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            management_object_ttl: DEFAULT_MANAGEMENT_TTL,
            gateway_memory_ttl: DEFAULT_GATEWAY_MEMORY_TTL,
            gateway_redis_ttl: None,
            user_api_key_cache_max_size: None,
        }
    }
}

#[derive(Debug, Error)]
enum SettingError {
    #[error("the value must be a positive whole number")]
    MaxSize,
    #[error("the value is not a duration the cache can hold")]
    Duration {
        #[source]
        source: TryFromFloatSecsError,
    },
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let management_object_ttl = match lookup.lookup(FALLBACK_VARIABLE) {
            None => DEFAULT_MANAGEMENT_TTL,
            Some(text) => seconds_from_integer(
                FALLBACK_VARIABLE,
                text.trim()
                    .parse::<i64>()
                    .map_err(|source| ConfigError::InvalidInteger {
                        path: FALLBACK_VARIABLE.to_owned(),
                        source,
                    })?,
            )?,
        };
        let configured = self
            .general_settings
            .user_api_key_cache_ttl
            .map(parse_ttl)
            .transpose()?;
        let max_size = self
            .general_settings
            .user_api_key_cache_max_size
            .map(parse_max_size)
            .transpose()?;
        Ok(Settings {
            management_object_ttl,
            gateway_memory_ttl: configured.unwrap_or(DEFAULT_GATEWAY_MEMORY_TTL),
            gateway_redis_ttl: configured,
            user_api_key_cache_max_size: max_size,
        })
    }
}

// int(os.getenv(...)) at import: Python's int takes a sign and digits, and an empty value fails it
fn seconds_from_integer(path: &str, parsed: i64) -> Result<f64, ConfigError> {
    parsed
        .to_string()
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: path.to_owned(),
            source,
        })
}

// the YAML scalar reduced to the value a cache setting reads, with bools and non-scalars rejected
enum NumericScalar {
    Whole(i64),
    Fractional(f64),
    Written(String),
}

fn numeric_scalar(raw: RawScalar, path: &str) -> Result<NumericScalar, ConfigError> {
    match raw {
        RawScalar::Integer(value) => Ok(NumericScalar::Whole(value)),
        RawScalar::Float(value) => Ok(NumericScalar::Fractional(value)),
        RawScalar::Text(text) => Ok(NumericScalar::Written(text)),
        RawScalar::Boolean(value) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value,
        }),
        RawScalar::Other(_) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
    }
}

fn parse_ttl(raw: RawScalar) -> Result<f64, ConfigError> {
    let seconds = match raw {
        RawScalar::Integer(value) => seconds_from_integer(TTL_PATH, value)?,
        RawScalar::Float(value) => value,
        RawScalar::Text(text) => {
            text.trim()
                .parse::<f64>()
                .map_err(|source| ConfigError::InvalidFloat {
                    path: TTL_PATH.to_owned(),
                    source,
                })?
        }
        RawScalar::Boolean(value) => {
            return Err(ConfigError::InvalidScalar {
                path: TTL_PATH.to_owned(),
                value,
            });
        }
        RawScalar::Other(_) => {
            return Err(ConfigError::InvalidScalar {
                path: TTL_PATH.to_owned(),
                value: false,
            });
        }
    };
    checked_seconds(seconds)
}

// zero, negative and fractional values stay as given; only what no Duration can hold is rejected
fn checked_seconds(seconds: f64) -> Result<f64, ConfigError> {
    Duration::try_from_secs_f64(seconds.abs())
        .map(|_duration| seconds)
        .map_err(|source| setting_error(TTL_PATH, &SettingError::Duration { source }))
}

fn parse_max_size(raw: RawScalar) -> Result<usize, ConfigError> {
    let scalar = numeric_scalar(raw, MAX_SIZE_PATH)?;
    let whole = whole_size(scalar)?;
    positive_size(whole)
}

// the scalar reduced to a whole number, so only a positive one becomes a cache size
fn whole_size(scalar: NumericScalar) -> Result<i64, ConfigError> {
    match scalar {
        NumericScalar::Whole(value) => Ok(value),
        NumericScalar::Fractional(value) => integral_float(value)
            .ok_or_else(|| setting_error(MAX_SIZE_PATH, &SettingError::MaxSize)),
        NumericScalar::Written(text) => {
            text.trim()
                .parse::<i64>()
                .map_err(|source| ConfigError::InvalidInteger {
                    path: MAX_SIZE_PATH.to_owned(),
                    source,
                })
        }
    }
}

fn positive_size(value: i64) -> Result<usize, ConfigError> {
    match usize::try_from(value) {
        Ok(0) | Err(_) => Err(setting_error(MAX_SIZE_PATH, &SettingError::MaxSize)),
        Ok(size) => Ok(size),
    }
}

// pydantic's int accepts a whole float and rejects a fraction
fn integral_float(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract().to_bits() != 0.0_f64.to_bits() {
        return None;
    }
    format!("{value:.0}").parse().ok()
}

fn setting_error(path: &str, error: &SettingError) -> ConfigError {
    ConfigError::Yaml {
        source: <serde_saphyr::Error as DeserializeError>::custom(format!("{path}: {error}")),
    }
}
