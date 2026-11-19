use betterlite_context::local_ttl_cache::CacheError;
use thiserror::Error;

use crate::routing_value_store::StoreError;

// the failures the callbacks log and swallow, and the ones a samples reader sees
#[derive(Debug, Error)]
pub enum LatencyError {
    #[error("a latency sample key is invalid")]
    Key {
        #[source]
        source: StoreError,
    },
    #[error("the latency sample cache failed")]
    Cache {
        #[from]
        source: CacheError,
    },
    #[error("the latency samples could not be encoded")]
    Encoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored latency samples could not be decoded")]
    Decoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored latency samples are not text")]
    UnexpectedValue,
    #[error("the logged times cannot be read as seconds")]
    Time,
    #[error("the clock value {seconds} is not a valid instant")]
    Clock { seconds: f64 },
    #[error("a latency minute counter overflowed")]
    Overflow,
}
