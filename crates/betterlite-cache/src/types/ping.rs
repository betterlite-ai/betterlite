use betterlite_models::decisions::Extra;
use serde::{Deserialize, Serialize};
use serde_json::Number;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TextOrNumber {
    Text(String),
    Number(Number),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CachePingResponse {
    pub status: String,
    pub cache_type: String,
    pub ping_response: Option<bool>,
    pub set_cache_response: Option<String>,
    pub litellm_cache_params: Option<String>,
    pub health_check_cache_params: Option<HealthCheckCacheParams>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCheckCacheParams {
    pub host: Option<String>,
    pub port: Option<TextOrNumber>,
    pub redis_kwargs: Option<Extra>,
    pub namespace: Option<String>,
    pub redis_version: Option<TextOrNumber>,
}
