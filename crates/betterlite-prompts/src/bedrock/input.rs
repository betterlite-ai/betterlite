use serde::Deserialize;
use thiserror::Error;

use crate::arguments::json::Json;
use crate::message_content::Message;

use super::assembly::AssistantContinue;
use super::flags::PromptCapabilities;

pub const MAX_PROMPT_INPUT_BYTES: usize = 0x0001_0000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptBedrockInput {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<Json>,
    pub model: Option<String>,
    #[serde(default)]
    pub capabilities: PromptCapabilities,
    pub user_continue_message: Option<Message>,
    pub assistant_continue_message: Option<AssistantContinue>,
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

pub fn parse_prompt_bedrock_input(bytes: &[u8]) -> Result<PromptBedrockInput, PromptInputError> {
    if bytes.len() > MAX_PROMPT_INPUT_BYTES {
        return Err(PromptInputError::TooLarge {
            length: bytes.len(),
            limit: MAX_PROMPT_INPUT_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| PromptInputError::Decode { source })
}
