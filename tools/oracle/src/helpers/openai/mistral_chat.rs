use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_choices::stream::StreamError;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::compatible_wire::{ReplyOptions, decode_compatible_chat_reply};
use betterlite_openai::mistral_chat::reply::decode_mistral_chat;
use betterlite_openai::mistral_chat::request::{RequestTransform, transform_request};
use betterlite_openai::mistral_chat::stream::chunk_parser;
use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Request {
        model: String,
        messages: Value,
        #[serde(default)]
        optional_params: Object,
    },
    Reply {
        reply: Value,
    },
    Chunk {
        chunk: Value,
    },
}

pub(crate) struct Op;

fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: String::from("MistralChatError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn stream_failure(error: StreamError) -> Box<HelperError> {
    let status = match &error {
        StreamError::Provider { status, .. } => Some(*status),
        StreamError::Chunk(_) | StreamError::ToolCallPayload => None,
    };
    Box::new(HelperError {
        class: String::from("StreamError"),
        status,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results: Vec<Value> = input
            .steps
            .iter()
            .map(step_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(results))
    }
}

fn step_value(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Request {
            model,
            messages,
            optional_params,
        } => request_value(model, messages, optional_params),
        Step::Reply { reply } => reply_value(reply),
        Step::Chunk { chunk } => chunk_value(chunk),
    }
}

fn request_value(
    model: &str,
    messages: &Value,
    optional_params: &Object,
) -> Result<Value, Box<HelperError>> {
    let messages: Vec<ChatMessage> = serde_json::from_value(messages.clone()).map_err(failure)?;
    let body = transform_request(&RequestTransform {
        model,
        messages: &messages,
        optional_params,
        downloads: &RefuseDownloads,
    })
    .map_err(failure)?;
    serde_json::to_value(body).map_err(failure)
}

fn reply_value(reply: &Value) -> Result<Value, Box<HelperError>> {
    let body = serde_json::to_vec(reply).map_err(failure)?;
    let decoded = decode_mistral_chat(&body).map_err(failure)?;
    let mutated = serde_json::to_vec(&decoded.value).map_err(failure)?;
    let converted = decode_compatible_chat_reply(
        &mutated,
        &ReplyOptions {
            json_mode: false,
            provider: None,
            base_model: None,
            identifier: Uuid::nil,
            clock: i64::default,
        },
    )
    .map_err(failure)?;
    let choice = converted.response.choices.first();
    let content = choice.and_then(|value| value.message.content.clone());
    Ok(json!({
        "message": {
            "content": content,
            "reasoning_content": decoded
                .value
                .pointer("/choices/0/message/reasoning_content")
                .cloned()
                .unwrap_or(Value::Null),
        },
        "response": {
            "model": converted.response.model,
            "finish_reason": choice.and_then(|value| value.finish_reason.clone()),
            "content": content,
            "tool_calls": choice.and_then(|value| value.message.tool_calls.as_ref().map(Vec::len)),
        },
    }))
}

fn chunk_value(chunk: &Value) -> Result<Value, Box<HelperError>> {
    let mut chunk = chunk.clone();
    let reply = chunk_parser(&mut chunk, Uuid::nil, i64::default).map_err(stream_failure)?;
    let choices: Vec<Value> = reply
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "delta": {
                    "content": choice.delta.content,
                    "reasoning_content": choice.delta.reasoning_content,
                    "thinking_blocks": choice
                        .delta
                        .passthrough
                        .get("thinking_blocks")
                        .cloned()
                        .unwrap_or(Value::Null),
                },
            })
        })
        .collect();
    Ok(json!({
        "object": reply.object,
        "choices": choices,
    }))
}
