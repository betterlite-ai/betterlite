use thiserror::Error;

use super::access::PollDenial;
use super::cache::PollingCacheError;
use super::decode::PollingDecodeError;

#[derive(Debug, Error)]
pub enum PollingError {
    #[error(transparent)]
    Cache(#[from] PollingCacheError),
    #[error(transparent)]
    Decode(#[from] PollingDecodeError),
    #[error(transparent)]
    Forbidden(#[from] PollDenial),
    #[error("the polling state could not be serialized: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
