use std::str::{Utf8Error, from_utf8};

use serde::{Deserialize, Serialize};

mod tests;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChirpCommand {
    Configure {
        model: String,
        language_codes: Vec<String>,
        sample_rate_hertz: i32,
    },
    FinishTurn {},
    DiscardTurn {},
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChirpEvent {
    Configured,
    TurnFinished,
    TurnDiscarded { billed_seconds: f64 },
}

#[derive(Debug, thiserror::Error)]
pub enum ChirpCommandError {
    #[error("the Speech-to-Text command is not UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("the Speech-to-Text command is not valid JSON")]
    Json(#[source] serde_json::Error),
}

pub fn decode_chirp_command(message: &[u8]) -> Result<ChirpCommand, ChirpCommandError> {
    let text = from_utf8(message).map_err(ChirpCommandError::Utf8)?;
    serde_json::from_str(text).map_err(ChirpCommandError::Json)
}
