use serde::Deserialize;

use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::request_destination::RequestData;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchPrecallInput {
    pub call: BatchPrecallCall,
}

// One batch request at the stage the proxy hooks run it: create, failed create, retrieve with its cache read, and terminal retrieve
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchPrecallCall {
    Create(BatchCreateCall),
    CreateFailure,
    Retrieve(BatchRetrieveCall),
    TerminalRetrieve(BatchTerminalCall),
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchCreateCall {
    pub call_type: String,
    pub principal: BatchLimitPrincipal,
    pub data: RequestData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchRetrieveCall {
    pub call_type: String,
    pub principal: BatchLimitPrincipal,
    pub data: RequestData,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub cache_control: Option<BatchCacheControl>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchTerminalCall {
    pub batch_id: String,
    pub status: String,
}

// The cache-control kwargs the response cache reads; litellm spells the bound s-max-age or s-maxage
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchCacheControl {
    #[serde(default)]
    pub max_age: Option<i64>,
}
