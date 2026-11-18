use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

const DEFAULT_TTL: i64 = 60;
const TTL_PATH: &str = "litellm_settings.default_redis_ttl";
const CACHE_PARAMS_TTL_PATH: &str = "litellm_settings.cache_params.default_redis_ttl";
const FLUSH_SIZE_PATH: &str = "litellm_settings.cache_params.redis_flush_size";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    cache: Option<ScalarBool>,
    default_redis_ttl: Option<RawNumber>,
    #[serde(default)]
    cache_params: RawCacheParams,
}

#[derive(Debug, Default, Deserialize)]
struct RawCacheParams {
    default_redis_ttl: Option<RawNumber>,
    redis_flush_size: Option<RawNumber>,
}

// the proxy tests `value is True`, so only a boolean turns the switch on and text leaves it off
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ScalarBool {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub default_ttl: i64,
    pub redis_flush_size: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_ttl: DEFAULT_TTL,
            redis_flush_size: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.litellm_settings;
        let cache_switch = match settings.cache {
            Some(ScalarBool::Boolean(value)) => value,
            Some(ScalarBool::Text(text)) => {
                let _text_length = text.len();
                false
            }
            None => false,
        };
        let (ttl_number, ttl_path) = if cache_switch {
            match settings.cache_params.default_redis_ttl {
                Some(number) => (Some(number), CACHE_PARAMS_TTL_PATH),
                None => (settings.default_redis_ttl, TTL_PATH),
            }
        } else {
            (settings.default_redis_ttl, TTL_PATH)
        };
        let default_ttl = match resolve_number(ttl_number, ttl_path, lookup, parse_float)? {
            Some(number) => cut_toward_zero(number, ttl_path)?,
            None => DEFAULT_TTL,
        };
        let redis_flush_size = if cache_switch {
            resolve_number(
                settings.cache_params.redis_flush_size,
                FLUSH_SIZE_PATH,
                lookup,
                parse_flush_size,
            )?
        } else {
            None
        };
        Ok(Settings {
            default_ttl,
            redis_flush_size,
        })
    }
}

// Python's int() cuts toward zero; -0 formats as "-0" and parses back to 0, and inf and NaN fail here
fn cut_toward_zero(value: f64, path: &str) -> Result<i64, ConfigError> {
    format!("{:.0}", value.trunc())
        .parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

fn parse_flush_size(text: &str, path: &str) -> Result<usize, ConfigError> {
    text.parse::<usize>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

#[path = "redis_cache/tests.rs"]
mod tests;
