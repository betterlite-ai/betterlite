use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use crate::files::error::StatusError;

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error("no API key is set for the speech call")]
    MissingApiKey,
    #[error("the api_base is not a URL")]
    BaseUrl {
        #[source]
        source: url::ParseError,
    },
    #[error("a speech request header name or value is not valid")]
    Header {
        #[source]
        source: http::Error,
    },
    #[error("the speech request could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the supplied client is {supplied} but this call needs the {needed} client")]
    ClientKind {
        supplied: &'static str,
        needed: &'static str,
    },
    #[error("the speech request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the speech logging hook failed")]
    Logging {
        #[source]
        source: PrePostError,
    },
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
    #[error("the speech reply body could not be read")]
    Body {
        #[source]
        source: HttpError,
    },
    #[error("the speech reply is larger than betterlite accepts")]
    TooLarge,
}
