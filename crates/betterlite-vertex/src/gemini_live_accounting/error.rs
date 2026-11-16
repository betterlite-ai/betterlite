use std::num::TryFromIntError;

use thiserror::Error;

use crate::gemini_live_setup::error::LiveSetupError;
use crate::gemini_usage::error::UsageError;

#[derive(Debug, Error)]
pub enum LiveAccountingError {
    #[error("a Gemini Live usage record could not be converted")]
    Convert(#[source] serde_json::Error),
    #[error(transparent)]
    Usage(#[from] UsageError),
    #[error(transparent)]
    Setup(#[from] LiveSetupError),
    #[error("the estimated transcription tokens do not fit a token count")]
    Estimate(#[from] TryFromIntError),
}
