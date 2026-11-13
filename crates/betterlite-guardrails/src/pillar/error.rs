use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;

#[derive(Debug, Error)]
pub enum PillarError {
    #[error(
        "Couldn't get Pillar API key, either set the `PILLAR_API_KEY` in the environment or pass it as a parameter to the guardrail in the config file"
    )]
    MissingApiKey,
    #[error("the Pillar API request failed")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("the Pillar API answered with status {status}")]
    Status { status: u16 },
    #[error("the Pillar API response body is not valid JSON")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Pillar API response body exceeds {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("a Pillar request header name is not valid")]
    InvalidHeaderName {
        #[source]
        source: reqwest::header::InvalidHeaderName,
    },
    #[error("a Pillar request header value is not valid")]
    InvalidHeaderValue {
        #[source]
        source: reqwest::header::InvalidHeaderValue,
    },
    #[error("the {field} field is not {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error("Unrecognised boolean value: {value}")]
    BoolValue { value: String },
    // The 400 and 503 LiteLLM raises, carried as the framework turns them into a response
    #[error(transparent)]
    Http(#[from] GuardrailFailure),
}
