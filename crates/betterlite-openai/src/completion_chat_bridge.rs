use std::collections::BTreeSet;
use std::sync::LazyLock;

use betterlite_models::catalog::BundledCatalog;
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::chat::deployment::{ChatDeployment, Provider};
use crate::chat::{ChatMessage, ChatRequest, MessageContent, Role};
use crate::text_completion_execution::catalog::CompletionCatalog;
use crate::text_completion_execution::failure::CompletionFailure;
use crate::text_completion_policy::SUPPORTED_PARAMS;
use crate::text_completion_request::{CompletionPrompt, CompletionRequest};
use crate::text_completion_transport::call::TextCompletionRequest as TransportRequest;

// main.py's dispatch sends these three provider tags to the text endpoint and converts the answer back
const TEXT_COMPLETION_PREFIXES: [&str; 3] = [
    "text-completion-openai/",
    "text-completion-codestral/",
    "text-completion-inception/",
];

// completion() sends the OpenAI fine-tuned completion models to the text endpoint without a conversion
const FINE_TUNED_COMPLETIONS: [&str; 2] = ["ft:babbage-002", "ft:davinci-002"];

// get_llm_provider sends a model in open_ai_text_completion_models to the text provider even untagged
static TEXT_COMPLETION_MODELS: LazyLock<BTreeSet<String>> =
    LazyLock::new(|| CompletionCatalog::new(&BundledCatalog::default().get()).text_models);

// Chat kwargs LiteLLM reads itself, so a rebuilt text request must not carry them to the provider
const CONTROL_PARAMS: [&str; 12] = [
    "api_base",
    "api_key",
    "api_version",
    "engine",
    "litellm_params",
    "metadata",
    "mock_delay",
    "mock_response",
    "mock_timeout",
    "mock_tool_calls",
    "num_retries",
    "stream_chunk_size",
];

const UNMAPPED_PROMPT: &str =
    "Unmapped prompt format. Your prompt is neither a list of strings nor a string.";

const TOKEN_PROMPT: &str = "a token-id prompt cannot be sent to this deployment's chat endpoint";

const MESSAGES_UNWRITTEN: &str = "the chat messages could not be written for the text endpoint";

const REQUEST_UNWRITTEN: &str = "the chat request could not be written for the text endpoint";

const INTERNAL_ERROR: &str = "internal_server_error";

// Where main.py's chat dispatch sends a request: the chat endpoint, or the text endpoint with or without the answer conversion
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatEndpoint {
    Chat,
    Text { convert: bool },
}

// Where main.py's text dispatch sends a request: the text endpoint, or a chat endpoint whose answer converts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionEndpoint {
    Text,
    Chat,
}

#[must_use]
pub fn chat_endpoint(deployment: &ChatDeployment) -> ChatEndpoint {
    // the provider tag itself is the prefix, and an untagged model of the set resolves to text-completion-openai
    if let Provider::Unsupported { model } = &deployment.provider
        && (TEXT_COMPLETION_PREFIXES
            .iter()
            .any(|prefix| model.starts_with(prefix))
            || TEXT_COMPLETION_MODELS.contains(model))
    {
        return ChatEndpoint::Text { convert: true };
    }
    // completion() tests the stripped model name, whichever provider resolved it
    let named = match &deployment.provider {
        Provider::OpenAi { model } | Provider::Unsupported { model } => model.as_str(),
    };
    if FINE_TUNED_COMPLETIONS
        .iter()
        .any(|fine| named.contains(fine))
    {
        return ChatEndpoint::Text { convert: false };
    }
    ChatEndpoint::Chat
}

#[must_use]
pub fn text_endpoint_model(deployment: &ChatDeployment) -> Option<String> {
    match chat_endpoint(deployment) {
        ChatEndpoint::Chat => None,
        ChatEndpoint::Text { .. } => Some(match &deployment.provider {
            Provider::Unsupported { model } => TEXT_COMPLETION_PREFIXES
                .iter()
                .find_map(|prefix| model.strip_prefix(prefix))
                .or_else(|| model.strip_prefix("openai/"))
                .map_or_else(|| model.clone(), str::to_owned),
            Provider::OpenAi { model } => model.clone(),
        }),
    }
}

pub fn text_request(
    deployment: &ChatDeployment,
    request: &ChatRequest,
    model: &str,
) -> Result<TransportRequest, CompletionFailure> {
    Ok(TransportRequest {
        model: Some(model.to_owned()),
        messages: Some(text_messages(request)?),
        api_key: deployment.api_key.clone(),
        api_base: Some(deployment.api_base.clone()),
        // the deployment timeout is the text call's deadline; the chat request names none
        timeout: deployment.timeout_seconds,
        // the text dispatch leaves the handler's organization at None
        organization: None,
        optional_params: text_params(request)?,
        headers: None,
        acompletion: true,
    })
}

fn text_messages(request: &ChatRequest) -> Result<Vec<Value>, CompletionFailure> {
    serde_json::to_value(&request.messages)
        .map_err(|source| failure(500, Some(INTERNAL_ERROR), &source.to_string()))?
        .as_array()
        .cloned()
        .ok_or_else(|| failure(500, Some(INTERNAL_ERROR), MESSAGES_UNWRITTEN))
}

// get_optional_params raises UnsupportedParamsError for a set value outside the text config's list
fn text_params(request: &ChatRequest) -> Result<Map<String, Value>, CompletionFailure> {
    let mut params = serde_json::to_value(request)
        .map_err(|source| failure(500, Some(INTERNAL_ERROR), &source.to_string()))?
        .as_object()
        .cloned()
        .ok_or_else(|| failure(500, Some(INTERNAL_ERROR), REQUEST_UNWRITTEN))?;
    params.remove("model");
    params.remove("messages");
    params.retain(|name, _| !CONTROL_PARAMS.contains(&name.as_str()) && !name.starts_with('_'));
    reject_unsupported(&params)?;
    set_stream(&mut params, request);
    Ok(params)
}

fn reject_unsupported(params: &Map<String, Value>) -> Result<(), CompletionFailure> {
    let Some(name) = params
        .keys()
        .find(|name| !SUPPORTED_PARAMS.contains(&name.as_str()))
    else {
        return Ok(());
    };
    Err(failure(
        400,
        Some("invalid_request_error"),
        &format!("the text endpoint does not support the parameter {name}"),
    ))
}

// An absent include_usage asks for usage the way the proxy does; a caller's own choice is kept.
fn set_stream(params: &mut Map<String, Value>, request: &ChatRequest) {
    if !request.stream {
        // main.py sends the chat dispatch's own stream flag, so the provider reads an explicit false
        params.insert(String::from("stream"), Value::Bool(false));
        return;
    }
    params.insert(String::from("stream"), Value::Bool(true));
    let chosen = request.stream_options.as_ref();
    let include_usage = chosen.is_none_or(|options| options.include_usage);
    let mut options = Map::new();
    options.insert(String::from("include_usage"), Value::Bool(include_usage));
    params.insert(String::from("stream_options"), Value::Object(options));
}

// The client's text prompt rebuilt as chat messages, one per string; typed messages cannot carry the token branch
pub fn chat_request(request: &CompletionRequest) -> Result<ChatRequest, CompletionFailure> {
    let messages = prompt_messages(request)?;
    let passthrough = passthrough_params(request)?;
    Ok(ChatRequest {
        model: request.model.clone(),
        messages,
        stream: request.stream == Some(true),
        stream_options: request.stream_options.clone(),
        temperature: request.temperature.clone(),
        top_p: request.top_p.clone(),
        presence_penalty: request.presence_penalty.clone(),
        frequency_penalty: request.frequency_penalty.clone(),
        choice_count: request.count.and_then(|count| u32::try_from(count).ok()),
        max_tokens: request
            .max_tokens
            .and_then(|tokens| u32::try_from(tokens).ok()),
        max_completion_tokens: None,
        seed: request.seed,
        stop: request.stop.clone(),
        // the chat field is a boolean flag, so the text request's depth rides the passthrough instead
        logprobs: None,
        top_logprobs: None,
        user: request.user.clone(),
        service_tier: None,
        reasoning_effort: None,
        tools: request.tools.clone(),
        tool_choice: request.tool_choice.clone(),
        parallel_tool_calls: None,
        response_format: request.response_format.clone(),
        fallbacks: None,
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        disable_fallbacks: None,
        passthrough,
    })
}

fn prompt_messages(request: &CompletionRequest) -> Result<Vec<ChatMessage>, CompletionFailure> {
    let mut messages = Vec::new();
    match &request.prompt {
        CompletionPrompt::Text(text) => messages.push(text.clone()),
        CompletionPrompt::Texts(texts) => messages.extend(texts.iter().cloned()),
        CompletionPrompt::Tokens(_) | CompletionPrompt::TokenLists(_) => {
            return Err(failure(400, Some("invalid_request_error"), TOKEN_PROMPT));
        }
        CompletionPrompt::Empty
        | CompletionPrompt::Unmapped(_)
        | CompletionPrompt::Unreadable(_) => {
            return Err(failure(500, None, UNMAPPED_PROMPT));
        }
    }
    Ok(messages
        .into_iter()
        .map(|text| ChatMessage {
            role: Role::User,
            content: Some(MessageContent::Text(text)),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        })
        .collect())
}

// text_completion() hands these five to the chat call, whose OpenAI config supports and forwards all of them
fn passthrough_params(
    request: &CompletionRequest,
) -> Result<IndexMap<String, Value>, CompletionFailure> {
    let params = request.optional_params().map_err(|source| {
        failure(
            500,
            Some(INTERNAL_ERROR),
            &format!("the text request could not be read as its fields: {source}"),
        )
    })?;
    let mut passthrough = IndexMap::new();
    for name in [
        "functions",
        "function_call",
        "logit_bias",
        "logprobs",
        "top_logprobs",
    ] {
        if let Some(value) = params.get(name).filter(|value| !value.is_null()) {
            passthrough.insert(String::from(name), value.clone());
        }
    }
    Ok(passthrough)
}

pub(super) fn failure(status: u16, error_type: Option<&str>, message: &str) -> CompletionFailure {
    CompletionFailure {
        status,
        error_type: error_type.map(str::to_owned),
        param: None,
        message: message.to_owned(),
    }
}

pub mod answer;
mod tests;
