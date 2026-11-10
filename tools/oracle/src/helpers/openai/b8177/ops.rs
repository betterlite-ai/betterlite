use betterlite_openai::gigachat_chat::config::GigachatChatConfig;
use betterlite_openai::gigachat_chat::headers::{HeaderInput, validate_environment};
use betterlite_openai::gigachat_chat::messages::transform_messages;
use betterlite_openai::gigachat_chat::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use betterlite_openai::gigachat_chat::reply::{ReplyInput, transform_response};
use betterlite_openai::gigachat_chat::request::{CompletionInput, completion_request};
use betterlite_openai::gigachat_stream::{StreamEvent, chunk_parser};
use secrecy::SecretString;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::helpers::HelperResult;

use super::input::{
    HeadersStep, MapParamsStep, MessagesStep, ReplyStep, RequestStep, Step, StreamStep,
};
use super::support::{
    FixedIds, TokenBearer, UnusedBearer, UnusedUploader, Uploader, chat_reply_value, failure,
    failure_without_status, stream_chunk_value, success,
};

pub(crate) fn execute(step: &Step, runtime: &Runtime) -> HelperResult {
    match step {
        Step::SupportedParams => Ok(success(vec![(
            "params",
            Value::Array(
                SUPPORTED_OPENAI_PARAMS
                    .iter()
                    .map(|name| Value::String((*name).to_owned()))
                    .collect(),
            ),
        )])),
        Step::MapParams(step) => Ok(map_params_value(step)),
        Step::Headers(step) => Ok(headers_value(runtime, step)),
        Step::Messages(step) => Ok(messages_value(runtime, step)),
        Step::Request(step) => Ok(request_value(runtime, step)),
        Step::Reply(step) => Ok(reply_value(step)),
        Step::Stream(step) => Ok(stream_value(step)),
    }
}

fn map_params_value(step: &MapParamsStep) -> Value {
    match map_openai_params(&step.params, step.optional_params.clone()) {
        Ok(params) => success(vec![("params", Value::Object(params))]),
        Err(error) => failure(error.status_code),
    }
}

fn headers_value(runtime: &Runtime, step: &HeadersStep) -> Value {
    let bearer = TokenBearer::new(&step.token);
    let key = step.api_key.as_deref().map(SecretString::from);
    let credentials = step.credentials.as_deref().map(SecretString::from);
    let fallback = step.fallback_key.as_deref().map(SecretString::from);
    let input = HeaderInput {
        api_key: key.as_ref(),
        credentials: credentials.as_ref(),
        fallback_key: fallback.as_ref(),
        caller_headers: &step.headers,
    };
    match runtime.block_on(validate_environment(&input, &bearer)) {
        Ok(headers) => success(vec![
            (
                "credentials",
                bearer.credentials().map_or(Value::Null, Value::String),
            ),
            (
                "headers",
                serde_json::to_value(headers).unwrap_or(Value::Null),
            ),
        ]),
        Err(error) => failure(error.status_code),
    }
}

fn messages_value(runtime: &Runtime, step: &MessagesStep) -> Value {
    let uploader = Uploader {
        ids: step.uploads.clone(),
    };
    match runtime.block_on(transform_messages(&step.messages, &uploader)) {
        Ok(messages) => success(vec![("messages", Value::Array(messages))]),
        Err(error) => failure(error.status_code),
    }
}

fn request_value(runtime: &Runtime, step: &RequestStep) -> Value {
    let bearer = TokenBearer::new(&step.token);
    let uploader = Uploader {
        ids: step.uploads.clone(),
    };
    let ids = FixedIds;
    let config = GigachatChatConfig {
        bearer: &bearer,
        uploader: &uploader,
        ids: &ids,
    };
    let input = CompletionInput {
        model: &step.model,
        messages: &step.messages,
        api_base: step.api_base.as_deref(),
        env_base: step.env_base.as_deref(),
        api_key: None,
        credentials: None,
        fallback_key: None,
        caller_headers: &step.headers,
        optional_params: &step.params,
    };
    match runtime.block_on(completion_request(&config, &input)) {
        Ok(request) => success(vec![
            ("url", Value::String(request.url)),
            (
                "headers",
                serde_json::to_value(request.headers).unwrap_or(Value::Null),
            ),
            ("body", Value::Object(request.body)),
        ]),
        Err(error) => failure(error.status_code),
    }
}

fn reply_value(step: &ReplyStep) -> Value {
    let bearer = UnusedBearer;
    let uploader = UnusedUploader;
    let ids = FixedIds;
    let config = GigachatChatConfig {
        bearer: &bearer,
        uploader: &uploader,
        ids: &ids,
    };
    let input = ReplyInput {
        status_code: step.status,
        body: step.body.as_bytes(),
        model: &step.model,
        created: step.created,
        structured_output: step.structured_output,
    };
    match transform_response(&config, &input) {
        Ok(response) => chat_reply_value(&response),
        Err(error) => failure(error.status_code),
    }
}

fn stream_value(step: &StreamStep) -> Value {
    let ids = FixedIds;
    let mut chunks = Vec::with_capacity(step.events.len());
    for event in &step.events {
        match chunk_parser(event.as_bytes(), &ids) {
            Ok(StreamEvent::Chunk(chunk)) => chunks.push(stream_chunk_value(&chunk)),
            Ok(StreamEvent::Done) => break,
            Err(_error) => return failure_without_status(),
        }
    }
    success(vec![("chunks", Value::Array(chunks))])
}
