use betterlite_anthropic::feature_flags::tools::is_native_web_search_tool;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

pub const STATUS_CODE: u16 = 400;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebSearchError {
    #[error(
        "Bedrock does not support Anthropic's server-side web search tool (tool type '{tool_type}', model '{model}'). To use web search with this model, enable LiteLLM's web search interception so the proxy executes the search instead: https://docs.litellm.ai/docs/integrations/websearch_interception. Alternatively, remove the web_search tool from the request."
    )]
    ServerSide { tool_type: String, model: String },
}

impl WebSearchError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::ServerSide { .. } => STATUS_CODE,
        }
    }
}

pub fn reject_unsupported_web_search_tools(
    request: &Object,
    model: &str,
) -> Result<(), WebSearchError> {
    let Some(Json::List(tools)) = request.get("tools") else {
        return Ok(());
    };
    for tool in tools {
        if is_native_web_search_tool(tool) {
            let tool_type = tool
                .as_object()
                .and_then(|fields| fields.get("type"))
                .and_then(Json::as_str)
                .unwrap_or_default();
            return Err(WebSearchError::ServerSide {
                tool_type: tool_type.to_owned(),
                model: model.to_owned(),
            });
        }
    }
    Ok(())
}
