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
    FinishTurn,
    DiscardTurn,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChirpEvent {
    Configured,
    TurnFinished,
    TurnDiscarded { billed_seconds: f64 },
}

impl TryFrom<&[u8]> for ChirpCommand {
    type Error = serde_json::Error;

    fn try_from(message: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(message)
    }
}
