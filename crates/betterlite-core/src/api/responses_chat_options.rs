#![expect(
    clippy::disallowed_types,
    reason = "the assembled messages are the heterogeneous JSON records LiteLLM's TypedDicts forward, and the bridge probe takes the serde_json values B158 declares"
)]

pub mod capabilities;
pub mod error;
pub mod reasoning;
pub mod records;
pub mod text_format;

mod tests;

use std::convert::Infallible;

use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use serde_json::Value;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, ItemsBoundary};
use crate::api::responses_codex_tools::tool_choices::chat_tool_choice;
use crate::api::responses_input_messages::{InputRequest, input_messages};
use crate::api::responses_tool_namespaces::chat_tools::{ChatTool, WebSearchOptions};
use crate::api::responses_tool_namespaces::responses_tools::responses_tools_to_chat;

use self::capabilities::{CompletionBridgeProbe, drops_web_search_options};
use self::error::ChatOptionsError;
use self::reasoning::{BridgeInputs, ReasoningChatForm, reasoning_for_chat};
use self::records::{ChatCompletionRequest, StreamOptions};
use self::text_format::response_format;

pub const SUPPORTED_RESPONSES_OPTIONS: [&str; 15] = [
    "input",
    "model",
    "instructions",
    "max_output_tokens",
    "metadata",
    "parallel_tool_calls",
    "previous_response_id",
    "reasoning",
    "stream",
    "temperature",
    "text",
    "tool_choice",
    "tools",
    "top_p",
    "user",
];

pub trait StreamOptionsSink {
    fn set_stream_options(&mut self, options: &StreamOptions);
}

pub struct RequestContext<'request> {
    pub request: &'request ResponsesApiRequestParams,
    pub provider: Option<&'request str>,
    pub stream: Option<bool>,
    pub extra_headers: Option<&'request Json>,
    pub metadata: Option<&'request Json>,
    pub service_tier: Option<&'request str>,
    pub api_base: Option<&'request str>,
}

pub struct RequestAssembly<'context> {
    pub limits: ApiLimits,
    pub cache: &'context DualLocalCache,
    pub parameters: &'context dyn SupportedParameterSource<Error = Infallible>,
    pub probe: &'context dyn CompletionBridgeProbe,
    pub logging: Option<&'context mut dyn StreamOptionsSink>,
    pub route_all_openai_to_responses: bool,
}

pub fn chat_completion_request(
    context: &RequestContext<'_>,
    assembly: &mut RequestAssembly<'_>,
) -> Result<ChatCompletionRequest, ChatOptionsError> {
    let request = context.request;
    let mapped = mapped_tools(
        request,
        context.provider,
        assembly.parameters,
        assembly.limits,
    )?;
    let messages = converted_messages(request, assembly)?;
    let reasoned = mapped_reasoning(request, context, &mapped, assembly)?;
    let stream_options = derived_stream_options(context.stream, assembly);
    let has_tools = !mapped.chat_tools.is_empty();
    Ok(ChatCompletionRequest {
        messages,
        model: request.model.clone(),
        tool_choice: tool_choice(request, has_tools),
        tools: has_tools.then_some(mapped.chat_tools),
        top_p: request.top_p.clone().flatten(),
        user: request.user.clone().flatten(),
        temperature: request.temperature.clone().flatten(),
        parallel_tool_calls: parallel_tool_calls(request, has_tools),
        max_tokens: request.max_output_tokens.flatten(),
        stream: context.stream,
        metadata: context
            .metadata
            .filter(|value| **value != Json::Null)
            .cloned(),
        service_tier: context.service_tier.map(str::to_owned),
        safety_identifier: request.safety_identifier.clone().flatten(),
        web_search_options: mapped.web_search_options,
        response_format: response_format(request.text.as_ref().and_then(Option::as_ref)),
        reasoning_effort: reasoned.effort,
        reasoning_summary: reasoned.summary,
        context_management: request.context_management.clone().flatten(),
        custom_llm_provider: context.provider.map(str::to_owned),
        extra_headers: context
            .extra_headers
            .filter(|value| **value != Json::Null)
            .cloned(),
        stream_options,
    })
}

struct MappedTools {
    chat_tools: Vec<ChatTool>,
    tool_values: Vec<Value>,
    web_search_options: Option<WebSearchOptions>,
    web_search_value: Option<Value>,
}

fn mapped_tools(
    request: &ResponsesApiRequestParams,
    provider: Option<&str>,
    parameters: &dyn SupportedParameterSource<Error = Infallible>,
    limits: ApiLimits,
) -> Result<MappedTools, ChatOptionsError> {
    let tools = tool_items(request.tools.as_ref().and_then(Option::as_ref), limits)?;
    let conversion = responses_tools_to_chat(&tools)?;
    let chat_tools = conversion.chat_tools;
    let web_search_options = conversion
        .web_search_options
        .filter(|_| !drops_web_search_options(&request.model, provider, parameters));
    let tool_values = chat_tools
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<Value>, _>>()?;
    let web_search_value = web_search_options
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    Ok(MappedTools {
        chat_tools,
        tool_values,
        web_search_options,
        web_search_value,
    })
}

fn converted_messages(
    request: &ResponsesApiRequestParams,
    assembly: &RequestAssembly<'_>,
) -> Result<Vec<Value>, ChatOptionsError> {
    let input = InputRequest {
        input: Some(request.input.clone()),
        instructions: request.instructions.clone().flatten(),
    };
    let messages = input_messages(&input, true, assembly.limits, assembly.cache)?;
    Ok(messages)
}

fn mapped_reasoning(
    request: &ResponsesApiRequestParams,
    context: &RequestContext<'_>,
    mapped: &MappedTools,
    assembly: &RequestAssembly<'_>,
) -> Result<ReasoningChatForm, ChatOptionsError> {
    let bridge = BridgeInputs {
        model: &request.model,
        custom_llm_provider: context.provider,
        web_search_options: mapped.web_search_value.as_ref(),
        tools: &mapped.tool_values,
        api_base: context.api_base,
        route_all_openai_to_responses: assembly.route_all_openai_to_responses,
    };
    reasoning_for_chat(
        request.reasoning.as_ref().and_then(Option::as_ref),
        &bridge,
        assembly.probe,
    )
}

fn derived_stream_options(
    stream: Option<bool>,
    assembly: &mut RequestAssembly<'_>,
) -> Option<StreamOptions> {
    if stream != Some(true) {
        return None;
    }
    let options = StreamOptions {
        include_usage: true,
    };
    if let Some(logging) = &mut assembly.logging {
        logging.set_stream_options(&options);
    }
    Some(options)
}

fn tool_items(tools: Option<&Json>, limits: ApiLimits) -> Result<Vec<Json>, ChatOptionsError> {
    match tools {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::List(items)) => {
            limits.check(ApiBoundariesInput::Items(ItemsBoundary {
                known_count: items.len(),
            }))?;
            Ok(items.clone())
        }
        Some(_) => Err(ChatOptionsError::WrongType { field: "tools" }),
    }
}

fn tool_choice(request: &ResponsesApiRequestParams, has_tools: bool) -> Option<Json> {
    if !has_tools {
        return None;
    }
    request
        .tool_choice
        .as_ref()
        .and_then(Option::as_ref)
        .map(chat_tool_choice)
}

const fn parallel_tool_calls(request: &ResponsesApiRequestParams, has_tools: bool) -> Option<bool> {
    if !has_tools {
        return None;
    }
    request.parallel_tool_calls.flatten()
}
