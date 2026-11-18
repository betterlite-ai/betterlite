use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

// RedisCache keeps this when its caller passes no flush size (redis_cache.py L655-L658).
const DEFAULT_FLUSH_SIZE: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: String,
    #[serde(default)]
    default_redis_ttl: Option<f64>,
    #[serde(default)]
    redis_flush_size: Option<usize>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "cache_defaults" => cache_defaults(input.default_redis_ttl, input.redis_flush_size),
            other => Err(Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: format!("unknown function: {other}"),
            })),
        }
    }
}

fn cache_defaults(default_redis_ttl: Option<f64>, redis_flush_size: Option<usize>) -> HelperResult {
    let mut cache_params = serde_json::Map::new();
    if let Some(ttl) = default_redis_ttl {
        cache_params.insert("default_redis_ttl".to_owned(), json!(ttl));
    }
    if let Some(flush_size) = redis_flush_size {
        cache_params.insert("redis_flush_size".to_owned(), json!(flush_size));
    }
    let yaml = json!({
        "litellm_settings": {
            "cache": true,
            "cache_params": Value::Object(cache_params),
        }
    })
    .to_string();
    let loaded = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&yaml)
    .or_config_error()?;
    let settings = loaded.sections.redis_cache;
    Ok(json!({
        "default_ttl": settings.default_ttl,
        "redis_flush_size": settings.redis_flush_size.unwrap_or(DEFAULT_FLUSH_SIZE),
    }))
}
