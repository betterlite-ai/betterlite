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

// no Result: Default bound, since a converted result has no empty value
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
impl<Result> LookupResponse<Result> {
    #[must_use]
    pub const fn cached(result: Result) -> Self {
        Self {
            cached_result: Some(result),
            final_embedding_response: None,
            all_embeddings_cached: false,
            embedding_uncached_input: None,
        }
    }

    #[must_use]
    pub const fn merged(
        response: Option<CachedEmbeddingResponse>,
        all_cached: bool,
        uncached: Vec<KeyValue>,
    ) -> Self {
        Self {
            cached_result: None,
            final_embedding_response: response,
            all_embeddings_cached: all_cached,
            embedding_uncached_input: Some(uncached),
        }
    }
}
