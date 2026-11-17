use thiserror::Error;

use crate::gemini_live_accounting::error::LiveAccountingError;
use crate::gemini_live_content::error::ContentError;
use crate::gemini_live_setup::error::LiveSetupError;
use crate::gemini_live_tool_turn::error::ToolTurnError;

#[derive(Debug, Error)]
pub enum LiveFrameError {
    #[error("the Gemini Live frame is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("Invalid JSON message: {message}")]
    InvalidJson {
        message: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Gemini Live frame is not a JSON object")]
    NotAnObject(#[source] serde_json::Error),
    #[error("`{path}` in the Gemini Live frame is not valid")]
    InvalidField {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("no betterlite event maps to the Gemini Live frame and its message is unknown")]
    UnknownMessage,
    #[error("a Gemini Live frame event could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error(transparent)]
    Accounting(#[from] LiveAccountingError),
    #[error(transparent)]
    ToolTurn(#[from] ToolTurnError),
    #[error(transparent)]
    Setup(#[from] LiveSetupError),
}
