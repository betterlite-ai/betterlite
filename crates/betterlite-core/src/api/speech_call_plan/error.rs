use betterlite_error::dispatch::{DispatchInput, dispatch};
use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::speech_call::error::{SpeechCallError, SpeechException, SpeechFailure, exception};
use crate::api::transcription_call::error::raw_exception;

#[derive(Debug, Error)]
pub enum AspeechError<Raised> {
    // aspeech reads kwargs["model"] before its try, so this KeyError reaches the caller unmapped
    #[error("'model'")]
    MissingModel,
    // a failure inside the try, with the model and provider exception_type maps it under
    #[error("aspeech failed for model {model} through provider {provider}")]
    Raised {
        model: String,
        provider: String,
        error: Raised,
    },
}

impl AspeechError<SpeechCallError> {
    // exception_type over a failure inside the try; the KeyError has no litellm class
    #[must_use]
    pub fn exception(&self, redactor: &Redactor) -> Option<SpeechException> {
        match self {
            Self::MissingModel => None,
            Self::Raised {
                model,
                provider,
                error,
            } => Some(mapped(error, model, provider, redactor)),
        }
    }
}

// speech()'s status-less failures reach exception_type too, and without a status it raises an APIConnectionError
fn mapped(
    error: &SpeechCallError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> SpeechException {
    match error {
        SpeechCallError::Sdk { message, .. }
        | SpeechCallError::Unmapped { message, .. }
        | SpeechCallError::Internal { message } => {
            let raw = raw_exception(message, None, model, provider);
            let class = dispatch(&DispatchInput::Raw(Box::new(raw)), redactor);
            SpeechException {
                failure: SpeechFailure::Mapped(class),
                message: message.clone(),
            }
        }
        other @ (SpeechCallError::Resolution(_)
        | SpeechCallError::BadRequest { .. }
        | SpeechCallError::Provider { .. }) => exception(other, model, provider, redactor),
    }
}
