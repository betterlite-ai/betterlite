use betterlite_anthropic::batches::decode::DecodeError;
use thiserror::Error;
use tokio::runtime::TryCurrentError;

#[derive(Debug, Error)]
pub(crate) enum AnthropicBatchError {
    #[error("the batch response body is not valid JSON: {source}")]
    Parse {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the batch response body is {kind}, which cannot be searched for an id")]
    UnsearchableBody { kind: &'static str },
    #[error("the batch managed object cannot be spawned without a Tokio runtime: {source}")]
    NoRuntime {
        #[source]
        source: TryCurrentError,
    },
}
