use serde::{Deserialize, Serialize};

mod tests;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

pub fn decode_chirp_command(message: &[u8]) -> Result<ChirpCommand, serde_json::Error> {
    let command: ChirpCommand = serde_json::from_slice(message)?;
    Ok(command)
}
