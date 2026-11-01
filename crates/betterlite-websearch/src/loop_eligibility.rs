use thiserror::Error;

use crate::call_transformation::wire::{Call, Response};
use crate::call_transformation::{TransformationError, WebSearchTransformation};
use crate::native_blocks::ResponseJson;
use crate::tools::{
    Tool, is_chat_web_search_tool, is_web_search_tool, is_web_search_tool_responses,
};

pub const CHAT_COMPLETION_SURFACE: &str = "chat_completions";
pub const RESPONSES_AGENTIC_SURFACE: &str = "responses";
const WEBSEARCH_TOOL_TYPE: &str = "websearch";

#[derive(Debug, Error)]
pub enum LoopEligibilityError {
    #[error("cannot parse response for search-loop eligibility")]
    Response(#[from] serde_json::Error),
    #[error(transparent)]
    Transformation(#[from] TransformationError),
}

#[derive(Debug, Clone, Default)]
pub struct WebSearchInterceptionState {
    pub enabled_providers: Option<Vec<String>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolsDict {
    pub tool_calls: Vec<Call>,
    pub tool_type: &'static str,
    pub provider: String,
    pub response_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<ResponseJson>>,
}

impl WebSearchInterceptionState {
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors LiteLLM's async_should_run_agentic_loop signature"
    )]
    pub fn async_should_run_agentic_loop(
        &self,
        response_json: &str,
        _model: &str,
        _messages: &[ResponseJson],
        tools: Option<&[Tool]>,
        stream: bool,
        custom_llm_provider: &str,
        agentic_loop_api_surface: Option<&str>,
    ) -> Result<Option<ToolsDict>, LoopEligibilityError> {
        match agentic_loop_api_surface {
            Some(CHAT_COMPLETION_SURFACE) => {
                self.chat_completion_gate(response_json, tools, stream, custom_llm_provider)
            }
            Some(RESPONSES_AGENTIC_SURFACE) => {
                self.responses_gate(response_json, tools, stream, custom_llm_provider)
            }
            _ => self.anthropic_gate(response_json, tools, stream, custom_llm_provider),
        }
    }

    pub fn anthropic_gate(
        &self,
        response_json: &str,
        tools: Option<&[Tool]>,
        stream: bool,
        custom_llm_provider: &str,
    ) -> Result<Option<ToolsDict>, LoopEligibilityError> {
        let has_search_tool = tools.unwrap_or(&[]).iter().any(is_web_search_tool);
        if !self.provider_allowed(custom_llm_provider) || !has_search_tool {
            return Ok(None);
        }
        let response: Response = serde_json::from_str(response_json)?;
        let (should_intercept, tool_calls) =
            WebSearchTransformation::transform_request(&response, stream, "anthropic")?;
        if !should_intercept {
            return Ok(None);
        }
        let raw: ResponseJson = serde_json::from_str(response_json)?;
        Ok(Some(ToolsDict {
            tool_calls,
            tool_type: WEBSEARCH_TOOL_TYPE,
            provider: custom_llm_provider.to_owned(),
            response_format: "anthropic",
            thinking_blocks: Some(extract_thinking_blocks(&raw)),
        }))
    }

    pub fn chat_completion_gate(
        &self,
        response_json: &str,
        tools: Option<&[Tool]>,
        stream: bool,
        custom_llm_provider: &str,
    ) -> Result<Option<ToolsDict>, LoopEligibilityError> {
        let has_search_tool = tools.unwrap_or(&[]).iter().any(is_chat_web_search_tool);
        if !self.provider_allowed(custom_llm_provider) || !has_search_tool {
            return Ok(None);
        }
        let response: Response = serde_json::from_str(response_json)?;
        let (should_intercept, tool_calls) =
            WebSearchTransformation::transform_request(&response, stream, "openai")?;
        if !should_intercept {
            return Ok(None);
        }
        Ok(Some(ToolsDict {
            tool_calls,
            tool_type: WEBSEARCH_TOOL_TYPE,
            provider: custom_llm_provider.to_owned(),
            response_format: "openai",
            thinking_blocks: None,
        }))
    }

    pub fn responses_gate(
        &self,
        response_json: &str,
        tools: Option<&[Tool]>,
        stream: bool,
        custom_llm_provider: &str,
    ) -> Result<Option<ToolsDict>, LoopEligibilityError> {
        let has_search_tool = tools
            .unwrap_or(&[])
            .iter()
            .any(is_web_search_tool_responses);
        if !self.provider_allowed(custom_llm_provider) || !has_search_tool {
            return Ok(None);
        }
        let response: Response = serde_json::from_str(response_json)?;
        let (should_intercept, tool_calls) =
            WebSearchTransformation::transform_request(&response, stream, "responses")?;
        if !should_intercept {
            return Ok(None);
        }
        Ok(Some(ToolsDict {
            tool_calls,
            tool_type: WEBSEARCH_TOOL_TYPE,
            provider: custom_llm_provider.to_owned(),
            response_format: "responses",
            thinking_blocks: None,
        }))
    }

    fn provider_allowed(&self, custom_llm_provider: &str) -> bool {
        let Some(providers) = self.enabled_providers.as_deref() else {
            return true;
        };
        providers
            .iter()
            .any(|provider| provider == custom_llm_provider)
    }
}

fn extract_thinking_blocks(response: &ResponseJson) -> Vec<ResponseJson> {
    let Some(content) = response.get("content").and_then(ResponseJson::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| {
            matches!(
                block.get("type").and_then(ResponseJson::as_str),
                Some("thinking" | "redacted_thinking")
            )
        })
        .cloned()
        .collect()
}
