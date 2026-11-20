use std::error::Error;
use std::io::Error as IoError;

use betterlite_config::tls::TlsPolicyError;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::error::ProviderError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

pub type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug, Error)]
#[error("litellm_params.{field} must be {expected}")]
pub struct FieldTypeError {
    pub field: &'static str,
    pub expected: &'static str,
}

#[derive(Debug, Error)]
pub enum VideoClientError {
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
    #[error(transparent)]
    Pool(#[from] PoolError),
}

// Everything that stops a request from reaching the provider, before its own error class is asked
#[derive(Debug, Error)]
pub enum WireFailure {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the multipart form field {field} must be a string, a number or a boolean")]
    Param { field: String },
}

// One edit try block's failures: each waits here until _handle_error turns it into the provider's error
#[derive(Debug, Error)]
pub enum StepFailure {
    #[error("{0}")]
    Config(BoxError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Wire(#[from] WireFailure),
}

impl From<BoxError> for StepFailure {
    fn from(error: BoxError) -> Self {
        Self::Config(error)
    }
}

// A provider config's error is opaque here, so it travels typed only as far as this module's boundary
#[derive(Debug, Error)]
pub enum VideoEditTransportError {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error(transparent)]
    Config(BoxError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Client(#[from] VideoClientError),
    #[error("the prefetch reply is not a JSON object")]
    PrefetchDecode(#[source] serde_json::Error),
    #[error(transparent)]
    FieldType(#[from] FieldTypeError),
    #[error("video sync handler cannot run inside a running runtime")]
    SyncInRuntime { status_code: u16 },
    #[error("the video sync handler could not build its runtime")]
    Runtime(#[source] IoError),
}
