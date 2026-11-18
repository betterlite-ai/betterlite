use betterlite_cache::cache_key::value::KeyValue;
use betterlite_cache::embedding_merge::response::CachedEmbeddingResponse;

// LiteLLM's CachingHandlerResponse, with the cache-hit result already converted for the caller
#[derive(Debug, PartialEq)]
pub struct LookupResponse<Result> {
    pub cached_result: Option<Result>,
    pub final_embedding_response: Option<CachedEmbeddingResponse>,
    pub all_embeddings_cached: bool,
    pub embedding_uncached_input: Option<Vec<KeyValue>>,
}

impl<Result> Default for LookupResponse<Result> {
    fn default() -> Self {
        Self {
            cached_result: None,
            final_embedding_response: None,
            all_embeddings_cached: false,
            embedding_uncached_input: None,
        }
    }
}
