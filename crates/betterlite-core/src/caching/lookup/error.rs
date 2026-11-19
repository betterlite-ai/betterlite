use std::error::Error;

use betterlite_cache::cache_key::error::CacheKeyError;
use betterlite_cache::embedding_merge::error::ProcessError;
use betterlite_cache::handler::HandlerError;
use betterlite_cache::replay::ReplayError;
use betterlite_models::resolution::ResolutionError;
use thiserror::Error;

use crate::caching::hit_callbacks::context::CacheHitError;

// A gateway token counter reports its own error, so the port hands it back boxed
#[derive(Debug, Error)]
pub enum TokenCountError {
    #[error("the response tokens could not be counted")]
    Counted {
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}
#[derive(Debug, Error)]
pub enum HitError {
    #[error("the {field} value must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the provider could not be resolved")]
    Resolution {
        #[source]
        source: ResolutionError,
    },
    #[error("the cache hit context could not be recorded")]
    Context {
        #[from]
        source: CacheHitError,
    },
    #[error("the response tokens could not be counted")]
    Tokens {
        #[from]
        source: TokenCountError,
    },
    #[error("the cache hit logging state was already taken by another caller")]
    Poisoned,
}

#[derive(Debug, Error)]
pub enum LookupError {
    #[error("the cache lookup request is invalid")]
    Request {
        #[from]
        source: HandlerError,
    },
    #[error("the cache key could not be derived")]
    Key {
        #[from]
        source: CacheKeyError,
    },
    #[error("the cached result could not be converted")]
    Replay {
        #[from]
        source: ReplayError,
    },
    #[error("the cache hit could not be logged")]
    Hit {
        #[from]
        source: HitError,
    },
    #[error("the cached embeddings could not be merged")]
    Embeddings {
        #[from]
        source: ProcessError<HitError>,
    },
    #[error("the cached embeddings could not be rendered")]
    Render {
        #[from]
        source: serde_json::Error,
    },
}
