use betterlite_prompts::response_tool_calls::projections::ToolCallResponse;
use serde::Deserialize;
use serde_json::value::RawValue;

use crate::primitives::json_value::JsonValue;

use super::error::RetrievalError;

pub(crate) struct ModelResponse {
    pub(crate) calls: ToolCallResponse,
    pub(crate) view: ResponseView,
}

// the three surfaces LiteLLM's extractors probe, each kept whole so a present list is told from a missing key
#[derive(Deserialize)]
pub(crate) struct ResponseView {
    #[serde(default)]
    pub(crate) choices: JsonValue,
    #[serde(default)]
    pub(crate) content: JsonValue,
    #[serde(default)]
    pub(crate) output: JsonValue,
}

pub(crate) enum ResponseFormat {
    Responses,
    Anthropic,
    Chat,
}

impl ModelResponse {
    pub(crate) fn parse(response: &RawValue) -> Result<Self, RetrievalError> {
        let encoded = response.get();
        let calls =
            serde_json::from_str(encoded).map_err(|source| RetrievalError::Response { source })?;
        let view =
            serde_json::from_str(encoded).map_err(|source| RetrievalError::Response { source })?;
        Ok(Self { calls, view })
    }

    pub(crate) const fn format(&self) -> ResponseFormat {
        if matches!(self.view.output, JsonValue::Array(_)) {
            ResponseFormat::Responses
        } else if matches!(self.view.content, JsonValue::Array(_)) {
            ResponseFormat::Anthropic
        } else {
            ResponseFormat::Chat
        }
    }
}
