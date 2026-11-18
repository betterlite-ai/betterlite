use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::{Deserialize, Serialize};

pub(crate) struct Op;

// RedisCache keeps this when its caller passes no flush size (redis_cache.py L655-L658).
const DEFAULT_FLUSH_SIZE: usize = 100;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    CacheDefaults,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    #[serde(default)]
    default_redis_ttl: Option<f64>,
    #[serde(default)]
    redis_flush_size: Option<usize>,
}

#[derive(Serialize)]
struct Config {
    litellm_settings: LitellmSettings,
}

#[derive(Serialize)]
struct LitellmSettings {
    cache: bool,
    cache_params: CacheParams,
}

#[derive(Serialize)]
struct CacheParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    default_redis_ttl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redis_flush_size: Option<usize>,
}

#[derive(Serialize)]
struct Output {
    default_ttl: i64,
    redis_flush_size: usize,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::CacheDefaults => cache_defaults(input),
        }
    }
}

fn cache_defaults(input: &Input) -> HelperResult {
    let config = Config {
        litellm_settings: LitellmSettings {
            cache: true,
            cache_params: CacheParams {
                default_redis_ttl: input.default_redis_ttl,
                redis_flush_size: input.redis_flush_size,
            },
        },
    };
    let yaml = serde_json::to_string(&config).or_value_error()?;
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&yaml)
    .or_config_error()?;
    let settings = loaded.sections.redis_cache;
    serde_json::to_value(Output {
        default_ttl: settings.default_ttl,
        redis_flush_size: settings.redis_flush_size.unwrap_or(DEFAULT_FLUSH_SIZE),
    })
    .or_value_error()
}
