pub mod config;
pub mod provider;
mod tests;
pub mod tool_rewrite;

use betterlite_anthropic::feature_flags::tools::is_native_web_search_tool;
use betterlite_config::agentic_loop_ceiling::AgenticLoopCeilingError;
use betterlite_config::sections::websearch_request_hooks::Settings;
use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;
use thiserror::Error;

use self::config::{ResolvedParams, WebSearchInterceptionParams};
use self::provider::ProviderError;
use self::tool_rewrite::{RewriteError, ToolFormat};

pub const EMIT_NATIVE_BLOCKS_KEY: &str = "_websearch_interception_emit_native_blocks";
pub const CONVERTED_STREAM_KEY: &str = "_websearch_interception_converted_stream";

const MAX_LOOPS: &str = "max_agentic_loops";
const STREAM: &str = "stream";
const TOOL_CHOICE: &str = "tool_choice";
const TOOLS: &str = "tools";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Ceiling(#[from] AgenticLoopCeilingError),
}

#[derive(Debug, Error)]
pub enum WebSearchError {
    #[error(transparent)]
    Rewrite(#[from] RewriteError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("stream must be a boolean")]
    Stream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSearchInterceptionLogger {
    pub enabled_providers: Option<Vec<String>>,
    pub search_tool_name: Option<String>,
    pub max_agentic_loops: Option<u32>,
}

impl From<Settings> for WebSearchInterceptionLogger {
    fn from(settings: Settings) -> Self {
        Self {
            enabled_providers: Some(settings.enabled_providers),
            search_tool_name: settings.search_tool_name,
            max_agentic_loops: settings.max_agentic_loops,
        }
    }
}

impl WebSearchInterceptionLogger {
    pub fn from_config_yaml(params: WebSearchInterceptionParams) -> Result<Self, ConfigError> {
        let ResolvedParams {
            enabled_providers,
            search_tool_name,
            max_agentic_loops,
        } = params.resolve()?;
        Ok(Self {
            enabled_providers,
            search_tool_name,
            max_agentic_loops,
        })
    }

    pub fn initialize_from_proxy_config(
        litellm_settings: Option<WebSearchInterceptionParams>,
        callback_specific_params: Option<WebSearchInterceptionParams>,
    ) -> Result<Self, ConfigError> {
        let Some(params) = litellm_settings.or(callback_specific_params) else {
            return Self::from_config_yaml(WebSearchInterceptionParams::default());
        };
        Self::from_config_yaml(params)
    }

    // The hook keeps LiteLLM's async name; the rewrite itself never waits
    pub fn async_pre_call_deployment_hook(
        &self,
        kwargs: &Object,
        call_type: Option<CallType>,
    ) -> Result<Option<Object>, WebSearchError> {
        let provider = provider::deployment_provider(kwargs)?;
        if !self.provider_enabled(provider.as_deref()) {
            return Ok(None);
        }
        let Some(tools) = request_tools(kwargs)? else {
            return Ok(None);
        };
        if matches!(call_type, Some(CallType::Responses | CallType::Aresponses)) {
            return responses_tools(kwargs, tools);
        }
        if !tool_rewrite::any_web_search_tool(tools)? {
            return Ok(None);
        }
        let converted = tool_rewrite::rewrite_tools(tools, ToolFormat::OpenAi)?;
        let mut request = kwargs.clone();
        if tools.iter().any(is_native_web_search_tool) {
            request.insert(EMIT_NATIVE_BLOCKS_KEY.to_owned(), Json::Bool(true));
        }
        request.insert(TOOLS.to_owned(), Json::List(converted));
        Ok(Some(stream_buffered(request)?))
    }

    pub fn async_pre_request_hook(
        &self,
        kwargs: &Object,
    ) -> Result<Option<Object>, WebSearchError> {
        let provider = provider::request_provider(kwargs)?;
        if !self.provider_enabled(provider.as_deref()) {
            return Ok(None);
        }
        let Some(tools) = request_tools(kwargs)? else {
            return Ok(None);
        };
        if !tool_rewrite::any_web_search_tool(tools)? {
            return Ok(None);
        }
        let converted = tool_rewrite::rewrite_tools(tools, ToolFormat::Anthropic)?;
        let mut request = kwargs.clone();
        if let Some(ceiling) = self.max_agentic_loops
            && matches!(request.get(MAX_LOOPS), None | Some(Json::Null))
        {
            request.insert(MAX_LOOPS.to_owned(), Json::Number(Number::from(ceiling)));
        }
        if tools.iter().any(is_native_web_search_tool) {
            request.insert(EMIT_NATIVE_BLOCKS_KEY.to_owned(), Json::Bool(true));
        }
        let choice = request
            .get(TOOL_CHOICE)
            .cloned()
            .map(|value| tool_rewrite::sync_forced_tool_choice(&value, &converted))
            .transpose()?;
        request.insert(TOOLS.to_owned(), Json::List(converted));
        if let Some(choice) = choice {
            request.insert(TOOL_CHOICE.to_owned(), choice);
        }
        Ok(Some(stream_buffered(request)?))
    }

    fn provider_enabled(&self, provider: Option<&str>) -> bool {
        let Some(enabled) = self.enabled_providers.as_deref() else {
            return true;
        };
        enabled
            .iter()
            .any(|name| name.as_str() == provider.unwrap_or_default())
    }
}

fn responses_tools(kwargs: &Object, tools: &[Json]) -> Result<Option<Object>, WebSearchError> {
    if !tool_rewrite::any_responses_web_search_tool(tools)? {
        return Ok(None);
    }
    let converted = tool_rewrite::rewrite_tools(tools, ToolFormat::Responses)?;
    let mut request = kwargs.clone();
    request.insert(TOOLS.to_owned(), Json::List(converted));
    Ok(Some(stream_buffered(request)?))
}

fn request_tools(kwargs: &Object) -> Result<Option<&[Json]>, WebSearchError> {
    match kwargs.get(TOOLS) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(tools)) if tools.is_empty() => Ok(None),
        Some(Json::List(tools)) => Ok(Some(tools.as_slice())),
        Some(_) => Err(WebSearchError::Rewrite(RewriteError::Tools)),
    }
}

fn stream_buffered(mut kwargs: Object) -> Result<Object, WebSearchError> {
    let stream = kwargs.get(STREAM).cloned();
    match stream {
        None | Some(Json::Null) | Some(Json::Bool(false)) => Ok(kwargs),
        Some(Json::Bool(true)) => {
            kwargs.insert(STREAM.to_owned(), Json::Bool(false));
            kwargs.insert(CONVERTED_STREAM_KEY.to_owned(), Json::Bool(true));
            Ok(kwargs)
        }
        Some(_) => Err(WebSearchError::Stream),
    }
}
