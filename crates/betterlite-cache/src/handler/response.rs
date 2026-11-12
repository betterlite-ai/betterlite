use betterlite_context::local_ttl_cache::CacheValue;

use crate::cache_key::value::KeyValue;
use crate::embedding::response::EmbeddingResponse;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CachingHandlerResponse {
    pub cached_result: Option<CacheValue>,
    pub final_embedding_response: Option<EmbeddingResponse>,
    pub all_embeddings_cached: bool,
    pub embedding_uncached_input: Option<Vec<KeyValue>>,
}
