use super::convert::{ConvertError, NativeErrorPayload};
use betterlite_config::env_integer::timeout::TimeoutError as RequestTimeoutError;
use betterlite_config::loader::ConfigError;
use betterlite_config::sections::vertex_credential_sources::credential_source::CredentialDecodeError;
use betterlite_env::EnvError;
use betterlite_http::error::HttpError;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_redact::error::RedactionError;
use betterlite_vertex::google_native_content::error::NativeContentError;
use pyo3::PyErr;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError, PyValueError};
// Every failure a generate content call raises comes back as this class, carrying the LiteLLM error payload as its text.
create_exception!(betterlite_native, NativeGenaiError, PyException);
// What the provider config reports while B3489 builds the body, the URL and the auth headers.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GenaiConfigError {
    #[error("the generate content request could not be built: {source}")]
    Native {
        #[source]
        source: NativeContentError,
    },
    #[error("the generate content body could not be encoded: {source}")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the vertex credentials could not be read: {source}")]
    Credentials {
        #[source]
        source: CredentialDecodeError,
    },
}
impl From<NativeContentError> for GenaiConfigError {
    fn from(source: NativeContentError) -> Self {
        Self::Native { source }
    }
}
// What a session needs from the process before it can take a call.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionError {
    #[error(transparent)]
    Json(#[from] ConvertError),
    #[error("the proxy environment could not be read: {source}")]
    Proxy {
        #[source]
        source: EnvError,
    },
    #[error("the outbound http client could not be built: {source}")]
    Client {
        #[source]
        source: HttpError,
    },
    #[error("the google settings could not be read: {source}")]
    Settings {
        #[source]
        source: ConfigError,
    },
    #[error("the request timeout could not be read: {source}")]
    Timeout {
        #[source]
        source: RequestTimeoutError,
    },
    #[error("the redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error("the call could not be logged: {source}")]
    Logging {
        #[source]
        source: LoggingStateError,
    },
}
impl From<ConvertError> for PyErr {
    fn from(error: ConvertError) -> Self {
        PyValueError::new_err(error.to_string())
    }
}
impl From<SessionError> for PyErr {
    fn from(error: SessionError) -> Self {
        match error {
            SessionError::Json(inner) => Self::from(inner),
            SessionError::Proxy { .. }
            | SessionError::Client { .. }
            | SessionError::Settings { .. }
            | SessionError::Timeout { .. }
            | SessionError::Redactor { .. }
            | SessionError::Logging { .. } => PyRuntimeError::new_err(error.to_string()),
        }
    }
}
// LiteLLM's exception_type names the class, and Python raises the matching type from the payload text.
pub(crate) fn payload_error(payload: &NativeErrorPayload) -> PyErr {
    match serde_json::to_string(payload) {
        Ok(text) => NativeGenaiError::new_err(text),
        Err(error) => PyRuntimeError::new_err(error.to_string()),
    }
}
