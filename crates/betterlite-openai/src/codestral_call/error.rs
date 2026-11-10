use crate::aleph_alpha::error::AlephAlphaError;
use crate::codestral_records::error::TextCompletionCodestralError;
use crate::error::OpenAiError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::error::FactoryError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::ParseError;
// LiteLLM wraps transport exceptions from the async call as TextCompletionCodestralError with status 500
pub const TRANSPORT_STATUS: u16 = 500;
#[derive(Debug, Error)]
pub enum CodestralCallError {
    #[error("no Codestral API key is set")]
    MissingApiKey,
    #[error("the Codestral prompt could not be built")]
    Prompt(#[from] FactoryError),
    #[error("the Codestral prompt selection returned no text")]
    PromptNotText,
    #[error("the Codestral settings could not be read")]
    Settings(#[from] OpenAiError),
    #[error("the {name} header name is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not valid")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Codestral URL is not valid")]
    Endpoint(#[source] ParseError),
    #[error("the Codestral request body could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Record(#[from] TextCompletionCodestralError),
    #[error("the Codestral provider could not be reached")]
    Transport {
        #[from]
        source: TransportError,
    },
    #[error("the Codestral line stream could not be read")]
    Lines {
        #[source]
        source: AlephAlphaError,
    },
}
