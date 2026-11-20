use std::convert::Infallible;
use std::error::Error;

use betterlite_context::media_optional_params::error::MediaParamsError;
use betterlite_error::dispatch::{DispatchInput, RawException, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::resolution::ResolutionError;
use betterlite_openai::api::audio_file_input::AudioFileError;
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::dispatch::error::DispatchError;

// what a downstream operation reports to exception_type: the text it reads and the raised message
pub trait TranscriptionDispatchError: Error + Send + Sync + 'static {
    fn status_code(&self) -> Option<u16> {
        None
    }

    // the text exception_type inspects, usually str(original_exception)
    fn error_str(&self) -> String {
        self.to_string()
    }

    // the text the raised exception carries
    fn message(&self) -> String {
        self.to_string()
    }
}

// transcription's own failures; every message is the one its ValueError carries in LiteLLM
#[derive(Debug, Error)]
pub enum TranscriptionError<OperationError = Infallible>
where
    OperationError: Error + 'static,
{
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Audio(#[from] AudioFileError),
    #[error(transparent)]
    Params(#[from] MediaParamsError),
    #[error("Unmapped provider passed in. Unable to get the response.")]
    UnmappedProvider,
    #[error(
        "Invalid response from transcription provider, expected TranscriptionResponse, but got {python_type}"
    )]
    InvalidResponse { python_type: &'static str },
    #[error("the cached transcription response is not a dict")]
    CachedResponseNotObject,
    #[error("the cached transcription response has a non-text {field}")]
    CachedField { field: String },
    #[error("the cached transcription response value is not JSON")]
    CachedValue {
        #[source]
        source: serde_json::Error,
    },
    #[error("timeout must be a non-negative number of seconds, got {seconds}")]
    InvalidTimeout { seconds: f64 },
    #[error("the transcription logging update failed")]
    Logging {
        #[from]
        source: LoggingStateError,
    },
    // main's dispatch stubs answer an unbuilt entry instead of falling through to another arm
    #[error(transparent)]
    NotBuilt(#[from] DispatchError),
    #[error(transparent)]
    Dispatch(OperationError),
}

// aimage_transcription's failure after litellm's exception_type mapped the sync exception
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionException {
    pub class: ErrorClass,
    pub message: String,
}

// the async wrapper's failure: a missing model is a KeyError raised before the try, so never mapped
#[derive(Debug)]
pub enum AsyncTranscriptionError {
    MissingModel,
    Exception(TranscriptionException),
}

// the three facts exception_type reads off a failure, whatever operation error it carries
struct FailureFacts {
    message: String,
    status: Option<u16>,
    error_str: String,
}

impl FailureFacts {
    fn from_source(status: Option<u16>, text: String) -> Self {
        Self {
            message: text.clone(),
            status,
            error_str: text,
        }
    }
}

impl<OperationError> TranscriptionError<OperationError>
where
    OperationError: Error + TranscriptionDispatchError,
{
    fn facts(&self) -> FailureFacts {
        match self {
            Self::Resolution(source) => FailureFacts::from_source(Some(400), source.to_string()),
            Self::Params(source) => {
                FailureFacts::from_source(Some(source.status_code()), source.to_string())
            }
            Self::Audio(source) => FailureFacts::from_source(None, source.to_string()),
            Self::Dispatch(source) => FailureFacts {
                message: source.message(),
                status: source.status_code(),
                error_str: source.error_str(),
            },
            Self::UnmappedProvider
            | Self::InvalidResponse { .. }
            | Self::CachedResponseNotObject
            | Self::CachedField { .. }
            | Self::CachedValue { .. }
            | Self::InvalidTimeout { .. }
            | Self::Logging { .. }
            | Self::NotBuilt(_) => FailureFacts::from_source(None, self.to_string()),
        }
    }
}

// Infallible stands for a failure the cached converter raises itself
impl TranscriptionDispatchError for Infallible {}

impl AsyncTranscriptionError {
    pub(crate) fn mapped<SourceError>(
        error: &TranscriptionError<SourceError>,
        model: &str,
        provider: &str,
        redactor: &Redactor,
    ) -> Self
    where
        SourceError: Error + TranscriptionDispatchError,
    {
        let facts = error.facts();
        Self::Exception(TranscriptionException {
            class: dispatch(
                &DispatchInput::Raw(Box::new(raw_exception(
                    &facts.error_str,
                    facts.status,
                    model,
                    provider,
                ))),
                redactor,
            ),
            message: facts.message,
        })
    }
}

// the attributes exception_type reads off the original exception before any provider branch runs
pub(crate) fn raw_exception(
    error_str: &str,
    status_code: Option<u16>,
    model: &str,
    provider: &str,
) -> RawException {
    RawException {
        model: model.to_owned(),
        custom_llm_provider: provider.to_owned(),
        error_str: error_str.to_owned(),
        status_code,
        status_code_is_synthesized: false,
        message: Some(error_str.to_owned()),
        body: None,
        exception_type: String::new(),
    }
}
