use betterlite_core::api::responses_input_messages::error::InputMessagesError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardrailRecordError {
    #[error("a guardrail record value could not be converted to or from JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the `messages` of a {call_type} request must be a list")]
    MessagesShape { call_type: String },
    #[error(transparent)]
    ResponsesInput(#[from] InputMessagesError),
}

#[derive(Debug, Error)]
pub enum GuardrailSpanError {
    #[error("the guardrail record could not be written as a span")]
    Encoding {
        #[source]
        source: serde_json::Error,
    },
    #[error("the span exporter is not accepting records")]
    Unavailable,
}
