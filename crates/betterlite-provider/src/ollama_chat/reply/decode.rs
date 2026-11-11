use serde::Deserialize;

use super::super::error::OllamaError;
use super::super::function::WireMessage;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DecodedReply {
    #[serde(default)]
    pub done_reason: Option<String>,
    pub message: WireMessage,
    #[serde(default)]
    pub prompt_eval_count: Option<u64>,
    #[serde(default)]
    pub eval_count: Option<u64>,
}

impl TryFrom<&[u8]> for DecodedReply {
    type Error = OllamaError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes).map_err(|source| OllamaError::Shape { source })
    }
}
