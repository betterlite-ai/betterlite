use std::sync::Arc;

use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::engine::Engine;
use crate::engine::huggingface::HuggingFaceEngine;

use super::error::CountError;
use super::request::{Message, SelectedTokenizer, TextInput, TokenRequest, ToolChoice};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum CustomTokenizerWire {
    #[serde(rename = "openai_tokenizer")]
    OpenAi,
    #[serde(rename = "huggingface_tokenizer")]
    HuggingFace { tokenizer_json: String },
}

// absent takes LiteLLM's default of False and adds the reply tokens; an explicit null is LiteLLM's None and skips them
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum ResponseTokens {
    Flag(bool),
    Null,
}

impl Default for ResponseTokens {
    fn default() -> Self {
        Self::Flag(false)
    }
}

impl ResponseTokens {
    #[must_use]
    pub const fn optional(self) -> Option<bool> {
        match self {
            Self::Flag(flag) => Some(flag),
            Self::Null => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenRequestWire {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub custom_tokenizer: Option<CustomTokenizerWire>,
    #[serde(default)]
    pub text: Option<TextInput>,
    #[serde(default)]
    pub messages: Option<Vec<Message>>,
    #[serde(default)]
    pub count_response_tokens: ResponseTokens,
    #[serde(default)]
    pub tools: Option<Vec<Json>>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
}

impl TryFrom<&[u8]> for TokenRequestWire {
    type Error = CountError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        serde_json::from_slice(bytes).map_err(|source| CountError::Json { source })
    }
}

impl TokenRequestWire {
    pub fn selected(&self) -> Result<Option<SelectedTokenizer>, CountError> {
        match &self.custom_tokenizer {
            None => Ok(None),
            Some(CustomTokenizerWire::OpenAi) => Ok(Some(SelectedTokenizer::OpenAi)),
            Some(CustomTokenizerWire::HuggingFace { tokenizer_json }) => {
                let engine = HuggingFaceEngine::from_bytes(tokenizer_json.as_bytes())?;
                Ok(Some(SelectedTokenizer::HuggingFace(Engine::HuggingFace(
                    Arc::new(engine),
                ))))
            }
        }
    }

    #[must_use]
    pub fn request<'wire>(
        &'wire self,
        custom_tokenizer: Option<&'wire SelectedTokenizer>,
    ) -> TokenRequest<'wire> {
        TokenRequest {
            model: &self.model,
            custom_tokenizer,
            text: self.text.as_ref(),
            messages: self.messages.as_deref(),
            count_response_tokens: self.count_response_tokens.optional(),
            tools: self.tools.as_deref(),
            tool_choice: self.tool_choice.as_ref(),
        }
    }
}
