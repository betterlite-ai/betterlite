use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponsesWsHistoryError {
    #[error("the {field} value must be {expected}")]
    Shape {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Send(#[from] FrameSinkError),
}

// LiteLLM swallows a failed error frame, so only the caller that sends a real frame reports this
#[derive(Debug, Error)]
#[error("the websocket text frame could not be sent: {message}")]
pub struct FrameSinkError {
    pub message: String,
}
