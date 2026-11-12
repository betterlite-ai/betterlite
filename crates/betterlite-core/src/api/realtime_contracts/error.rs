use betterlite_models::decisions::Json;
use std::collections::BTreeMap;
use thiserror::Error;
pub type HeaderMap = BTreeMap<String, Json>;
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct WebSocketError {
    pub message: String,
}
#[derive(Debug, Error)]
pub enum RealtimeContractError {
    #[error("the realtime JSON is {observed} bytes; the maximum is {limit}")]
    JsonTooLarge { observed: usize, limit: usize },
    #[error("unknown realtime contract kind: {0}")]
    UnknownKind(u8),
    #[error("the realtime JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the realtime record does not match its shape: {0}")]
    Record(#[from] serde_path_to_error::Error<serde_json::Error>),
    #[error("the realtime error event could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the client websocket failed: {0}")]
    Socket(#[from] WebSocketError),
}
