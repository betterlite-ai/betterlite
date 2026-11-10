use serde::Deserialize;

use super::value::{KeyObject, KeyValue};

// LiteLLM's **kwargs in the order the caller built them; the file entry carries the uploaded file's name, the only part of the file object the key reads.
#[derive(Debug, Deserialize)]
pub struct CacheKeyRequest {
    #[serde(default)]
    pub params: Vec<KeyParam>,
    pub metadata: Option<KeyObject>,
    pub litellm_metadata: Option<KeyObject>,
    pub litellm_params: Option<LiteLlmParams>,
    pub cache_control: Option<RequestCacheControl>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyParam {
    pub name: String,
    pub value: KeyValue,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LiteLlmParams {
    pub preset_cache_key: Option<String>,
    pub metadata: Option<KeyObject>,
    pub litellm_metadata: Option<KeyObject>,
    pub file_name: Option<KeyValue>,
}

// The ttls and no-cache/no-store keys of the request's cache dict belong to the cache-policy tasks, so only what key derivation reads is typed here.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RequestCacheControl {
    pub namespace: Option<String>,
    #[serde(default, rename = "use-cache")]
    pub use_cache: bool,
}
