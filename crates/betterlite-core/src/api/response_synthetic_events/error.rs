use thiserror::Error;
#[derive(Debug, Error)]
pub enum SyntheticEventsError {
    #[error("range() arg 3 must not be zero")]
    ZeroChunkSize,
    #[error("synthetic event payload could not be converted: {0}")]
    Payload(#[from] serde_json::Error),
    #[error("{field} must be a JSON object")]
    NotAnObject { field: &'static str },
    #[error("synthetic response events would exceed the size budget")]
    EventBudgetExceeded,
}
