use serde::Deserialize;
use thiserror::Error;

use crate::arguments::json::Json;

pub const MAX_PROMPT_INPUT_BYTES: usize = 0x0001_0000;

#[derive(Debug, Deserialize)]
pub struct PromptAnthropicInput {
    pub messages: Vec<Json>,
    pub model: String,
    pub llm_provider: String,
    #[serde(default)]
    pub modify_params: bool,
}

#[derive(Debug, Error)]
pub enum PromptInputError {
    #[error("the input is {length} bytes, over the {limit} byte limit")]
    TooLarge { length: usize, limit: usize },
    #[error("the input is not valid JSON")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

pub fn parse_prompt_anthropic_input(
    bytes: &[u8],
) -> Result<PromptAnthropicInput, PromptInputError> {
    if bytes.len() > MAX_PROMPT_INPUT_BYTES {
        return Err(PromptInputError::TooLarge {
            length: bytes.len(),
            limit: MAX_PROMPT_INPUT_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| PromptInputError::Decode { source })
}
