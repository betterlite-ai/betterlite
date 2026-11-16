use std::collections::BTreeMap;

use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use betterlite_openai::chat::ToolCall;
use betterlite_openai::chat_choices::stream::{StreamError, chunk_parser};
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::fireworks_ai_chat::auth::{FireworksChatEnv, validate_environment};
use betterlite_openai::fireworks_ai_chat::error::FireworksChatError;
use betterlite_openai::fireworks_ai_chat::models::{decode_models_response, get_models_request};
use betterlite_openai::fireworks_ai_chat::reply::{Reply, ReplyInput, transform_response};
use betterlite_openai::fireworks_ai_chat::request::{RequestInput, transform_request};
use betterlite_openai::fireworks_ai_common::{FireworksEnv, resolve_api_key};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::input::{
    CatalogModel, ChunkCall, HeaderCall, Input, KeyCall, ModelCall, ReplyCall, RequestCall,
};

const TOOLS: &str = "tools";
const FIXED_UUID: u128 = 1;
const FIXED_TIME: i64 = 1_700_000_000;

pub(crate) fn run(input: &Input) -> HelperResult {
    let catalog = load_catalog(&input.catalog);
    Ok(json!({
        "headers": header_values(&input.headers)?,
        "keys": key_values(&input.keys),
        "requests": request_values(&input.requests, &catalog)?,
        "replies": reply_values(&input.replies)?,
        "chunks": chunk_values(&input.chunks)?,
        "models": model_values(&input.models)?,
    }))
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

fn header_values(calls: &[HeaderCall]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let env = chat_env(&call.environment);
        let key = call.api_key.as_deref().map(SecretString::from);
        let resolved =
            validate_environment(key.as_ref(), &call.headers, &call.litellm_params, &env)
                .map_err(|error| helper_error(&error))?;
        values.push(as_value(&resolved)?);
    }
    Ok(values)
}

fn key_values(calls: &[KeyCall]) -> Vec<Value> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let env = chat_env(&call.environment);
        let key = call.api_key.as_deref().map(SecretString::from);
        let resolved = resolve_api_key(key.as_ref(), &env.keys)
            .map(|secret| secret.expose_secret().to_owned());
        values.push(json!(resolved));
    }
    values
}

fn request_values(
    calls: &[RequestCall],
    catalog: &Catalog,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let environment: BTreeMap<String, String> = BTreeMap::new();
        let input = RequestInput {
            model: &call.model,
            messages: &call.messages,
            optional_params: &call.optional_params,
            catalog,
            environment: &environment,
            downloads: &RefuseDownloads,
        };
        let body = transform_request(&input).map_err(|error| helper_error(&error))?;
        values.push(as_value(&body)?);
    }
    Ok(values)
}

fn reply_values(calls: &[ReplyCall]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let input = ReplyInput {
            status: call.status,
            headers: &call.headers,
            body: call.body.as_bytes(),
            tools: call.optional_params.get(TOOLS),
            identifier: || Uuid::from_u128(FIXED_UUID),
            clock: || FIXED_TIME,
        };
        let reply = transform_response(&input).map_err(|error| helper_error(&error))?;
        values.push(reply_value(&reply)?);
    }
    Ok(values)
}

fn reply_value(reply: &Reply) -> Result<Value, Box<HelperError>> {
    let response = &reply.response;
    let mut choices = Vec::with_capacity(response.choices.len());
    for choice in &response.choices {
        choices.push(json!({
            "index": choice.index,
            "finish_reason": choice.finish_reason,
            "content": choice.message.content,
            "tool_calls": tool_calls_value(choice.message.tool_calls.as_deref()),
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
        "hidden": as_value(&reply.hidden_params)?,
    }))
}

fn tool_calls_value(calls: Option<&[ToolCall]>) -> Value {
    let Some(calls) = calls else {
        return Value::Null;
    };
    Value::Array(
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
            .collect(),
    )
}

fn chunk_values(calls: &[ChunkCall]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let chunk = chunk_parser(&call.chunk, || Uuid::from_u128(FIXED_UUID), || FIXED_TIME)
            .map_err(stream_error)?;
        let reply = &chunk.reply;
        let mut choices = Vec::with_capacity(reply.choices.len());
        for choice in &reply.choices {
            choices.push(json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "role": choice.delta.role,
                "content": choice.delta.content,
            }));
        }
        values.push(json!({
            "id": reply.id,
            "object": reply.object,
            "created": reply.created,
            "model": reply.model,
            "choices": choices,
            "provider_specific_fields": as_value(&chunk.provider_specific_fields)?,
        }));
    }
    Ok(values)
}

fn model_values(calls: &[ModelCall]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let env = chat_env(&call.environment);
        let key = call.api_key.as_deref().map(SecretString::from);
        let request = get_models_request(call.api_base.as_deref(), key.as_ref(), &env)
            .map_err(|error| helper_error(&error))?;
        let models = decode_models_response(call.status, call.body.as_bytes())
            .map_err(|error| helper_error(&error))?;
        values.push(json!({
            "models": models,
            "url": request.url,
            "authorization": request.headers.get("Authorization"),
        }));
    }
    Ok(values)
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

fn as_value<T: Serialize>(value: &T) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(value).map_err(|source| {
        Box::new(HelperError {
            class: String::from("Serialize"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: source.to_string(),
        })
    })
}

fn helper_error(error: &FireworksChatError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("FireworksChatError"),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn stream_error(error: StreamError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("StreamError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
