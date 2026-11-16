use std::collections::BTreeMap;

use betterlite_models::decisions::HiddenParams;
use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat_choices::stream::StreamError;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::fireworks_ai_chat::auth::{FireworksChatEnv, validate_environment};
use betterlite_openai::fireworks_ai_chat::chunk::chunk_parser;
use betterlite_openai::fireworks_ai_chat::error::FireworksChatError;
use betterlite_openai::fireworks_ai_chat::models::{decode_models_response, get_models_request};
use betterlite_openai::fireworks_ai_chat::reply::{Reply, ReplyInput, transform_response};
use betterlite_openai::fireworks_ai_chat::request::{RequestInput, transform_request};
use betterlite_openai::fireworks_ai_common::{FireworksEnv, resolve_api_key};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use super::input::{
    CatalogModel, ChunkCall, HeaderCall, Input, KeyCall, ModelCall, ReplyCall, RequestCall,
};

const TOOLS: &str = "tools";
const FIXED_UUID: u128 = 1;
const FIXED_TIME: i64 = 1_700_000_000;
const CHAT_ERROR: &str = "FireworksChatError";
const STREAM_ERROR: &str = "StreamError";
const SERIALIZE_ERROR: &str = "Serialize";

// one case converts one operation, so each failure keeps the class and status its own error carries
#[derive(Debug, Error)]
enum OperationError {
    #[error(transparent)]
    Chat(#[from] FireworksChatError),
    #[error(transparent)]
    Stream(#[from] StreamError),
    #[error("the value could not be serialized: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl OperationError {
    fn helper(self) -> Box<HelperError> {
        let (class, status, message) = match self {
            Self::Chat(error) => (CHAT_ERROR, error.status_code(), error.to_string()),
            Self::Stream(error) => (STREAM_ERROR, None, error.to_string()),
            Self::Serialize(error) => (SERIALIZE_ERROR, None, error.to_string()),
        };
        Box::new(HelperError {
            class: class.to_owned(),
            status,
            error_type: None,
            param: None,
            code: None,
            message,
        })
    }
}

pub(crate) fn run(input: &Input) -> HelperResult {
    let catalog = load_catalog(&input.catalog);
    // a key lookup reads the environment only, so it never fails and needs no error to carry
    let keys: Vec<Value> = input.keys.iter().map(key_value).collect();
    Ok(json!({
        "headers": collect(&input.headers, header_value)?,
        "keys": keys,
        "requests": collect(&input.requests, |call| request_value(call, &catalog))?,
        "replies": collect(&input.replies, reply_value)?,
        "chunks": collect(&input.chunks, chunk_value)?,
        "models": collect(&input.models, model_value)?,
    }))
}

// every operation reads its own list, so one pass converts one call and the caller keeps the order
fn collect<T, F>(calls: &[T], convert: F) -> Result<Vec<Value>, Box<HelperError>>
where
    F: Fn(&T) -> Result<Value, OperationError>,
{
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(convert(call).map_err(OperationError::helper)?);
    }
    Ok(values)
}

fn load_catalog(models: &[CatalogModel]) -> Catalog {
    let mut entries = Catalog::new();
    for model in models {
        if let Ok((fields, _warnings)) = ModelFields::parse(&model.model_info) {
            entries.insert(model.key.clone(), fields);
        }
    }
    entries
}

fn header_value(call: &HeaderCall) -> Result<Value, OperationError> {
    let env = chat_env(&call.environment);
    let key = call.api_key.as_deref().map(SecretString::from);
    let resolved = validate_environment(key.as_ref(), &call.headers, &call.litellm_params, &env)?;
    Ok(serde_json::to_value(&resolved)?)
}

fn key_value(call: &KeyCall) -> Value {
    let env = chat_env(&call.environment);
    let key = call.api_key.as_deref().map(SecretString::from);
    let resolved =
        resolve_api_key(key.as_ref(), &env.keys).map(|secret| secret.expose_secret().to_owned());
    json!(resolved)
}

fn request_value(call: &RequestCall, catalog: &Catalog) -> Result<Value, OperationError> {
    let environment = BTreeMap::new();
    let input = RequestInput {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        catalog,
        environment: &environment,
        downloads: &RefuseDownloads,
    };
    let body = transform_request(&input)?;
    Ok(serde_json::to_value(&body)?)
}

fn reply_value(call: &ReplyCall) -> Result<Value, OperationError> {
    let input = ReplyInput {
        status: call.status,
        headers: &call.headers,
        body: call.body.as_bytes(),
        tools: call.optional_params.get(TOOLS),
        identifier: || Uuid::from_u128(FIXED_UUID),
        clock: || FIXED_TIME,
    };
    let reply = transform_response(&input)?;
    reply_json(&reply)
}

// the completion fields a case compares, with the usage counts and the Fireworks hidden params
fn reply_json(reply: &Reply) -> Result<Value, OperationError> {
    let response = &reply.response;
    let mut choices = Vec::with_capacity(response.choices.len());
    for choice in &response.choices {
        choices.push(json!({
            "index": choice.index,
            "finish_reason": choice.finish_reason,
            "content": choice.message.content,
            "tool_calls": choice.message.tool_calls.as_deref().map(tool_calls_value),
            "native_finish_reason": choice.provider_specific_fields.get("native_finish_reason"),
        }));
    }
    Ok(json!({
        "id": response.id,
        "object": response.object,
        "created": response.created,
        "model": response.model,
        "choices": choices,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
        "hidden": serde_json::to_value(&reply.hidden_params)?,
    }))
}

fn chunk_value(call: &ChunkCall) -> Result<Value, OperationError> {
    let chunk = chunk_parser(&call.chunk, || Uuid::from_u128(FIXED_UUID), || FIXED_TIME)?;
    let reply = &chunk.reply;
    chunk_json(reply, chunk.provider_specific_fields.as_ref())
}

// the chunk fields a case compares, plus the provider details the handler keeps off the reply
fn chunk_json(
    reply: &StreamReply,
    provider_fields: Option<&HiddenParams>,
) -> Result<Value, OperationError> {
    let choices: Vec<Value> = reply
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "role": choice.delta.role,
                "content": choice.delta.content,
            })
        })
        .collect();
    Ok(json!({
        "id": reply.id,
        "object": reply.object,
        "created": reply.created,
        "model": reply.model,
        "choices": choices,
        "provider_specific_fields": serde_json::to_value(provider_fields)?,
    }))
}

fn model_value(call: &ModelCall) -> Result<Value, OperationError> {
    let env = chat_env(&call.environment);
    let key = call.api_key.as_deref().map(SecretString::from);
    let request = get_models_request(call.api_base.as_deref(), key.as_ref(), &env)?;
    let models = decode_models_response(call.status, call.body.as_bytes())?;
    Ok(json!({
        "models": models,
        "url": request.url,
        "authorization": request.headers.get("Authorization"),
    }))
}

fn tool_calls_value(calls: &[ToolCall]) -> Vec<Value> {
    calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "type": call.kind,
                "function": {
                    "name": call.function.name,
                    "arguments": call.function.arguments,
                },
            })
        })
        .collect()
}

fn chat_env(environment: &BTreeMap<String, String>) -> FireworksChatEnv {
    FireworksChatEnv {
        keys: FireworksEnv {
            fireworks_api_key: configured(environment, "FIREWORKS_API_KEY").map(SecretString::from),
            fireworks_ai_api_key: configured(environment, "FIREWORKS_AI_API_KEY")
                .map(SecretString::from),
            fireworksai_api_key: configured(environment, "FIREWORKSAI_API_KEY")
                .map(SecretString::from),
            fireworks_ai_token: configured(environment, "FIREWORKS_AI_TOKEN")
                .map(SecretString::from),
        },
        api_base: configured(environment, "FIREWORKS_API_BASE"),
        account_id: configured(environment, "FIREWORKS_ACCOUNT_ID"),
    }
}

fn configured(environment: &BTreeMap<String, String>, name: &str) -> Option<String> {
    environment
        .get(name)
        .filter(|value| !value.is_empty())
        .cloned()
}
