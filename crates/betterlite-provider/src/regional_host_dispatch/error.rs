use betterlite_openai::minimax_speech::error::MinimaxSpeechError;

use crate::hosted_platform_dispatch::handler::HandlerError;
use thiserror::Error;

// main.py's regional arms: a failed handler, a ValueError the arm raises before any call, and a body that cannot be written
#[derive(Debug, Error)]
pub enum DispatchError {
    #[error(transparent)]
    Handler(#[from] HandlerError),
    #[error("{message}")]
    MissingKey { message: &'static str },
    #[error(transparent)]
    Speech(Box<MinimaxSpeechError>),
    #[error("the call could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
