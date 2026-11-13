use std::time::Duration;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_choices::stream::StreamError;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::compatible_wire::{LogInput, ReplyOptions};
use betterlite_openai::mistral_chat::reply::{
    ResponseReply, decode_mistral_chat, transform_response,
};
use betterlite_openai::mistral_chat::request::{RequestTransform, transform_request};
use betterlite_openai::mistral_chat::stream::chunk_parser;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::{HelperError, HelperResult};

mod tests;

const INIT: &str = r#"{"model":"mistral-small-latest","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Request(RequestStep),
    Reply(ReplyStep),
    Chunk(ChunkStep),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestStep {
    model: String,
    messages: Value,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyStep {
    reply: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkStep {
    chunk: Value,
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(failure)?;
        let results: Vec<Value> = input
            .steps
            .iter()
            .map(|step| match step {
                Step::Request(request) => request_step(request),
                Step::Reply(reply) => reply_step(reply, &redactor),
                Step::Chunk(chunk) => chunk_step(chunk),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Array(results))
    }
}

fn request_step(step: &RequestStep) -> Result<Value, Box<HelperError>> {
    let messages: Vec<ChatMessage> =
        serde_json::from_value(step.messages.clone()).map_err(failure)?;
    let body = transform_request(&RequestTransform {
        model: &step.model,
        messages: &messages,
        optional_params: &step.optional_params,
        downloads: &RefuseDownloads,
    })
    .map_err(failure)?;
    serde_json::to_value(body).map_err(failure)
}

fn reply_step(step: &ReplyStep, redactor: &Redactor) -> Result<Value, Box<HelperError>> {
    let (mut state, env) = fixture(redactor)?;
    let body = serde_json::to_vec(&step.reply).map_err(failure)?;
    let decoded = decode_mistral_chat(&body).map_err(failure)?;
    let headers = Headers::new();
    let converted = transform_response(
        &ResponseReply {
            status: 200,
            headers: &headers,
            body: &body,
        },
        &mut state,
        &env,
        LogInput {
            messages: Json::Null,
            data: RawBody::Object(IndexMap::new()),
        },
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
            "reasoning_content": mutated_reasoning(&decoded.value),
        },
        "response": {
            "model": converted.response.model,
            "finish_reason": choice.and_then(|value| value.finish_reason.clone()),
            "content": content,
            "tool_calls": choice.and_then(|value| value.message.tool_calls.as_ref().map(Vec::len)),
            "usage": [
                converted.response.usage.prompt_tokens,
                converted.response.usage.completion_tokens,
                converted.response.usage.total_tokens
            ],
        }
    }))
}

fn mutated_reasoning(value: &Value) -> Value {
    value
        .pointer("/choices/0/message/reasoning_content")
        .cloned()
        .unwrap_or(Value::Null)
}

fn chunk_step(step: &ChunkStep) -> Result<Value, Box<HelperError>> {
    let mut chunk = step.chunk.clone();
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

fn fixture(redactor: &Redactor) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).map_err(failure)?,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(failure)?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::<FixedOffset>::parse_from_rfc3339(NOW).map_err(failure)?,
    };
    Ok((state, env))
}
