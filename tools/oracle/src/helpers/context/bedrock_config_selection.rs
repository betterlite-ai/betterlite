use betterlite_bedrock::aws_endpoints::provider::{
    BedrockInvokeProvider as SourceProvider, invoke_provider,
};
use betterlite_bedrock::model_routes::{
    BedrockRoute as SourceRoute, CLAUDE_PLATFORM_PREFIX, CONVERSE_PREFIX, MANTLE_PREFIX,
    ModelRouteSelector, ParamMap, has_route_prefix,
};
use betterlite_context::bedrock_config_selection::chat::{
    BedrockInvokeProvider, BedrockRoute, select_bedrock_chat_config,
};
use betterlite_context::bedrock_config_selection::legacy::LEGACY_ANTHROPIC_MODEL_NAMES;
use betterlite_context::bedrock_config_selection::messages::{
    ExplicitRoute, select_bedrock_messages_config,
};
use betterlite_context::bedrock_model::get_bedrock_base_model;
use indexmap::IndexSet;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ChatConfigSelector { models: Vec<String> },
    NativeMessagesRoute { models: Vec<String> },
    LegacyAnthropicNames,
    SelectionWithoutConstruction { models: Vec<String> },
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

const fn route_of(route: SourceRoute) -> BedrockRoute {
    match route {
        SourceRoute::Converse => BedrockRoute::Converse,
        SourceRoute::Invoke => BedrockRoute::Invoke,
        SourceRoute::ClaudePlatform => BedrockRoute::ClaudePlatform,
        SourceRoute::ConverseLike => BedrockRoute::ConverseLike,
        SourceRoute::Agent => BedrockRoute::Agent,
        SourceRoute::Agentcore => BedrockRoute::Agentcore,
        SourceRoute::AsyncInvoke => BedrockRoute::AsyncInvoke,
        SourceRoute::Openai => BedrockRoute::Openai,
        SourceRoute::Mantle => BedrockRoute::Mantle,
        SourceRoute::ChatCompletions => BedrockRoute::ChatCompletions,
    }
}

const fn provider_of(provider: SourceProvider) -> BedrockInvokeProvider {
    match provider {
        SourceProvider::Cohere => BedrockInvokeProvider::Cohere,
        SourceProvider::Anthropic => BedrockInvokeProvider::Anthropic,
        SourceProvider::Mistral => BedrockInvokeProvider::Mistral,
        SourceProvider::Amazon => BedrockInvokeProvider::Amazon,
        SourceProvider::Meta => BedrockInvokeProvider::Meta,
        SourceProvider::Llama => BedrockInvokeProvider::Llama,
        SourceProvider::Ai21 => BedrockInvokeProvider::Ai21,
        SourceProvider::Nova => BedrockInvokeProvider::Nova,
        SourceProvider::DeepseekR1 => BedrockInvokeProvider::DeepseekR1,
        SourceProvider::Qwen3 => BedrockInvokeProvider::Qwen3,
        SourceProvider::Qwen2 => BedrockInvokeProvider::Qwen2,
        SourceProvider::Twelvelabs => BedrockInvokeProvider::Twelvelabs,
        SourceProvider::Openai => BedrockInvokeProvider::Openai,
        SourceProvider::Stability => BedrockInvokeProvider::Stability,
        SourceProvider::Moonshot => BedrockInvokeProvider::Moonshot,
    }
}

fn chat_report(model: &str) -> Result<Value, Box<HelperError>> {
    let price_map = ParamMap::new();
    let converse_models = IndexSet::new();
    let selector = ModelRouteSelector::new(&price_map, &converse_models, false)
        .map_err(|error| invalid(&error.to_string()))?;
    let route = selector
        .route(model, None)
        .map_err(|error| invalid(&error.to_string()))?;
    let provider = invoke_provider(model).map(provider_of);
    let base_model = get_bedrock_base_model(model);
    let tag = select_bedrock_chat_config(route_of(route), provider, &base_model);
    Ok(json!({"class": tag.class_name(), "arguments": []}))
}

fn messages_report(model: &str) -> Value {
    let mut explicit = Vec::new();
    if has_route_prefix(model, CLAUDE_PLATFORM_PREFIX) {
        explicit.push(ExplicitRoute::ClaudePlatform);
    }
    if has_route_prefix(model, CONVERSE_PREFIX) {
        explicit.push(ExplicitRoute::Converse);
    }
    if has_route_prefix(model, MANTLE_PREFIX) {
        explicit.push(ExplicitRoute::Mantle);
    }
    select_bedrock_messages_config(model, &explicit).map_or(
        Value::Null,
        |tag| json!({"class": tag.class_name(), "arguments": []}),
    )
}

fn chat_reports(models: &[String]) -> Result<Value, Box<HelperError>> {
    let mut reports = Vec::with_capacity(models.len());
    for model in models {
        reports.push(chat_report(model)?);
    }
    Ok(Value::Array(reports))
}

fn construction_reports(models: &[String]) -> Result<Value, Box<HelperError>> {
    let mut reports = Vec::with_capacity(models.len());
    for model in models {
        let chat = chat_report(model)?;
        let messages = messages_report(model);
        reports.push(json!({"chat": chat, "messages": messages}));
    }
    Ok(Value::Array(reports))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::ChatConfigSelector { models } => chat_reports(models),
            Input::NativeMessagesRoute { models } => Ok(Value::Array(
                models.iter().map(|model| messages_report(model)).collect(),
            )),
            Input::LegacyAnthropicNames => Ok(json!(LEGACY_ANTHROPIC_MODEL_NAMES)),
            Input::SelectionWithoutConstruction { models } => construction_reports(models),
        }
    }
}

mod tests;
