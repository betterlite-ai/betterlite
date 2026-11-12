use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_context::local_ttl_cache::CacheValue;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Eligibility(Eligibility),
    CurrentFormatEmbeddingEntry(Entries),
    Retrieve(Retrieve),
}

#[derive(Deserialize)]
pub(crate) struct Eligibility {
    pub(crate) call_type: String,
    pub(crate) supported_call_types: Option<Vec<CachingSupportedCallType>>,
    pub(crate) cache: KeyValue,
}

#[derive(Deserialize)]
pub(crate) struct Entries {
    pub(crate) entries: Vec<CacheValue>,
}

#[derive(Deserialize)]
pub(crate) struct Retrieve {
    pub(crate) call_type: CachingSupportedCallType,
    pub(crate) kwargs: KeyObject,
    pub(crate) parameters: Vec<String>,
    pub(crate) args: Vec<KeyValue>,
    pub(crate) entries: Vec<SeedEntry>,
}

#[derive(Deserialize)]
pub(crate) struct SeedEntry {
    pub(crate) input: KeyValue,
    pub(crate) record: CacheValue,
}
