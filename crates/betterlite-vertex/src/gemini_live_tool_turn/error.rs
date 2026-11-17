use thiserror::Error;

use crate::gemini_live_accounting::error::LiveAccountingError;

#[derive(Debug, Error)]
pub enum ToolTurnError {
    #[error("the usage of a Gemini Live tool turn could not be converted")]
    Usage(#[from] LiveAccountingError),
    #[error("the usage of a Gemini Live tool turn could not be encoded")]
    Encode(#[from] serde_json::Error),
}
