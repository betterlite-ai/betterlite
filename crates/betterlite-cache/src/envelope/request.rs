use serde::Deserialize;

use crate::cache_key::request::CacheKeyRequest;
use crate::types::control::DynamicCacheControl;

// One cache call's kwargs: the cache_key shortcut, the key-derivation request, and the cache dict the envelope reads.
#[derive(Debug, Deserialize)]
pub struct EnvelopeRequest {
    pub cache_key: Option<String>,
    pub key: CacheKeyRequest,
    #[serde(default)]
    pub control: DynamicCacheControl,
}
