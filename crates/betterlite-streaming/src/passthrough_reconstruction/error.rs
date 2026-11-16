use std::convert::Infallible;
use std::error::Error as StdError;
use std::str::Utf8Error;

use thiserror::Error;

use crate::streamed_response_assembly::AssemblyError;

#[derive(Debug, Error)]
pub enum EventError {
    #[error("the relayed chunk is not valid UTF-8")]
    Utf8(#[from] Utf8Error),
    #[error("the collapsed event could not be encoded as JSON")]
    Encode(#[from] serde_json::Error),
}

// Malformed marks the event a converter answered with a JSON decode error, which LiteLLM skips.
#[derive(Debug, Error)]
pub enum ConvertError<Failure = Infallible>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error("the relayed SSE event is not JSON")]
    Malformed,
    #[error("the relayed SSE event ended the stream")]
    Ended,
    #[error("the relayed SSE event could not be converted")]
    Failed {
        #[source]
        source: Failure,
    },
}

#[derive(Debug, Error)]
pub enum ReconstructionError<Failure = Infallible>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Event(#[from] EventError),
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
    #[error("the relayed SSE event could not be converted")]
    Conversion {
        #[source]
        source: Failure,
    },
    #[error("the usage object could not be encoded as JSON")]
    Encode(#[from] serde_json::Error),
    #[error("the relayed usage could not be calculated")]
    Usage {
        #[source]
        source: Failure,
    },
}
