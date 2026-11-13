use std::str::Utf8Error;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::files::error::StatusError;

#[derive(Debug, Error)]
pub enum TranscriptionError {
    #[error("the transcription API key is not set")]
    MissingApiKey,
    #[error("the api_base is not a URL")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("the transcription field `{field}` {problem}")]
    Field {
        field: String,
        problem: &'static str,
    },
    #[error("the transcription API key is not a header value")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the transcription request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the transcription logging hook failed")]
    Logging {
        #[source]
        source: PrePostError,
    },
    #[error("the transcription reply is not UTF-8 text")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the transcription value is not JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the transcription reply is not a JSON object")]
    NotObject,
    #[error("the transcription reply has no text or usage field")]
    NoResponseFields,
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
}
