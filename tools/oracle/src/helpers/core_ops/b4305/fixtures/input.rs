use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_context::local_ttl_cache::CacheValue;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenarios: Vec<Scenario>,
}

// One _async_get_cache or _sync_get_cache call: the request, the logging setup and the cache entries already stored
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    pub(crate) operation: Operation,
    pub(crate) model: String,
    pub(crate) call_type: CachingSupportedCallType,
    pub(crate) original_function: String,
    pub(crate) supported_call_types: Option<Vec<CachingSupportedCallType>>,
    pub(crate) cache_configured: bool,
    pub(crate) cache_type: LiteLlmCacheType,
    pub(crate) kwargs: KeyObject,
    #[serde(default)]
    pub(crate) entries: Vec<Seed>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Async,
    Sync,
}

// A stored value under a literal key, under an embedding entry key for one input, or under the key the request derives
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Seed {
    #[serde(default)]
    pub(crate) key: Option<String>,
    #[serde(default)]
    pub(crate) input: Option<KeyValue>,
    pub(crate) value: CacheValue,
}
