use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioIdentityError {
    #[error(
        "GEMINI_API_BASE or GEMINI_API_KEY/GOOGLE_API_KEY is not set. Please set the environment variable, to query Gemini's `/models` endpoint."
    )]
    MissingCredentials,
    #[error("Failed to fetch models from Gemini. Status code: {status}, Response: {body}")]
    Status { status: u16, body: String },
    #[error("Gemini model list is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("Gemini model list is not a valid catalog: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("the Gemini model list URL is not valid")]
    InvalidUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header holds a character a header cannot carry")]
    InvalidHeader {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Gemini model list request failed")]
    Transport {
        #[source]
        source: TransportError,
    },
}
