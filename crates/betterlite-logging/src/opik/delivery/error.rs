use std::error::Error as StdError;

use super::native::NativeClientError;
use super::record::RecordError;

#[derive(Debug, thiserror::Error)]
pub enum OpikError<E>
where
    E: StdError + 'static,
{
    #[error("the Opik request failed")]
    Transport(#[source] E),
    #[error("the Opik request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Opik API answered status {status} for a synchronous send, expected 204: {detail}")]
    UnexpectedStatus { status: u16, detail: String },
    #[error(transparent)]
    Payload(#[from] RecordError),
    #[error(transparent)]
    Native(#[from] NativeClientError),
}
