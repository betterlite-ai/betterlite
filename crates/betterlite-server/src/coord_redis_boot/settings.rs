use betterlite_config::sections::settings::JsonValue;
use serde::Deserialize;
use serde::de::Error as _;

use super::error::CoordRedisError;
use crate::proxy_request_contracts::coordination_redis::BoolOrText;

// The two config sections this module reads; every other key in them belongs to another module.
#[derive(Debug, Default)]
pub struct CoordinationRedisSettings {
    pub general_settings: CoordinationGeneralSettings,
    pub litellm_settings: CoordinationLitellmSettings,
}

// A section that is null reads as absent, the way docs/compatibility.md treats every null, so the raw read keeps each one optional.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawCoordinationRedisSettings {
    general_settings: Option<CoordinationGeneralSettings>,
    litellm_settings: Option<CoordinationLitellmSettings>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CoordinationGeneralSettings {
    pub coordination_redis: Option<CoordinationRedisBlock>,
    pub use_redis_transaction_buffer: Option<BoolOrText>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CoordinationLitellmSettings {
    pub enable_redis_auth_cache: Option<bool>,
}

// The block's fields stay unmodeled until CoordinationRedisParams validates them, so a non-mapping block is reported where it enters startup.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct CoordinationRedisBlock {
    pub value: JsonValue,
}

impl CoordinationRedisSettings {
    // The typed boundary: the proxy config mapping becomes the two sections this module consumes.
    pub fn from_config(config: &JsonValue) -> Result<Self, CoordRedisError> {
        let outcome: Result<
            RawCoordinationRedisSettings,
            serde_path_to_error::Error<serde_json::Error>,
        > = serde_path_to_error::deserialize(config.clone());
        let raw = outcome.map_err(settings_error)?;
        Ok(Self {
            general_settings: raw.general_settings.unwrap_or_default(),
            litellm_settings: raw.litellm_settings.unwrap_or_default(),
        })
    }
}

fn settings_error(error: serde_path_to_error::Error<serde_json::Error>) -> CoordRedisError {
    let path = error.path().to_string();
    let source = error.into_inner();
    let message = if path == "." {
        source.to_string()
    } else {
        format!("{path}: {source}")
    };
    CoordRedisError::Settings {
        source: serde_json::Error::custom(message),
    }
}
