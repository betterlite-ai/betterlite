use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_records::error::GuardrailRecordError;

#[derive(Debug, Error)]
pub enum EnkryptAiError {
    #[error("the EnkryptAI request could not be sent")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the EnkryptAI request timed out")]
    Timeout {
        #[source]
        source: reqwest::Error,
    },
    #[error("EnkryptAI answered with HTTP status {status}")]
    HttpStatus { status: u16 },
    #[error("the EnkryptAI response exceeds {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the EnkryptAI response is not JSON")]
    ResponseJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the EnkryptAI response is not a JSON object")]
    MalformedResponse,
    #[error("the EnkryptAI response does not match the detection shape")]
    ResponseShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the EnkryptAI {name} header is not a valid header value")]
    InvalidHeader {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the EnkryptAI guardrail detected attacks: {message}")]
    Detected { message: String },
    #[error("the EnkryptAI guardrail record could not be written")]
    Record {
        #[source]
        source: GuardrailRecordError,
    },
    #[error("the EnkryptAI guardrail settings are invalid")]
    Base {
        #[source]
        source: GuardrailBaseError,
    },
    #[error("the EnkryptAI response could not be converted")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}
