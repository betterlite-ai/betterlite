use betterlite_redact::secret::Redactor;
use thiserror::Error;

use crate::api::speech_call::error::{SpeechCallError, SpeechException, exception};

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
            } => Some(exception(error, model, provider, redactor)),
        }
    }
}
