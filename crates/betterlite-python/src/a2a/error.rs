use betterlite_a2a::errors::A2AError;
use betterlite_config::loader::ConfigError;
use betterlite_config::tls::TlsPolicyError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_http::tls::TlsError;
use betterlite_redact::error::RedactionError;
use betterlite_tokenizer::count::error::CountError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::PyException;

use super::payload::ErrorPayload;

create_exception!(betterlite_native, NativeA2aError, PyException);

#[derive(Debug, thiserror::Error)]
pub(crate) enum A2aCallError {
    #[error("the native a2a request text is not JSON of the expected shape: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the a2a answer could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the a2a card text is not JSON of the expected shape: {source}")]
    Card {
        #[source]
        source: serde_json::Error,
    },
    #[error("request is required")]
    MissingRequest,
    #[error("request is required for completion bridge")]
    MissingBridgeRequest,
    #[error("the a2a settings field '{field}' is not a valid number of seconds")]
    Timeout { field: &'static str },
    #[error("the a2a extra header name {name:?} is not valid: {source}")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the a2a extra header value for '{name}' is not valid: {source}")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the a2a proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the default configuration could not be loaded: {source}")]
    Config {
        #[source]
        source: ConfigError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the a2a TLS policy could not be resolved: {source}")]
    Policy {
        #[source]
        source: TlsPolicyError,
    },
    #[error("the a2a TLS policy could not be applied: {source}")]
    Tls {
        #[source]
        source: TlsError,
    },
    #[error("the tokenizer counter could not be built: {source}")]
    Tokenizer {
        #[source]
        source: CountError,
    },
    #[error("the redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error(transparent)]
    A2A(#[from] A2AError),
}

impl A2aCallError {
    // A malformed call or setting is the caller's mistake; the environment and encoding failures are ours.
    pub(crate) fn payload(&self) -> ErrorPayload {
        match self {
            Self::A2A(source) => ErrorPayload::from_a2a(source),
            Self::Json { .. }
            | Self::Card { .. }
            | Self::MissingRequest
            | Self::MissingBridgeRequest
            | Self::Timeout { .. }
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. } => ErrorPayload::value_error(self.to_string()),
            Self::Encode { .. }
            | Self::Proxy { .. }
            | Self::Config { .. }
            | Self::Client { .. }
            | Self::Policy { .. }
            | Self::Tls { .. }
            | Self::Tokenizer { .. }
            | Self::Redactor { .. } => ErrorPayload::runtime_error(self.to_string()),
        }
    }
}

impl From<A2aCallError> for PyErr {
    fn from(error: A2aCallError) -> Self {
        error.payload().into_exception()
    }
}
