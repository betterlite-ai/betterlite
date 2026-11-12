use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::message_content::Message;
use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_provider::ollama_chat::reply::{OllamaChatResponse, ToolCall};
use betterlite_provider::ollama_chat::usage::{TokenCounter, Usage};
use betterlite_provider::ollama_generate::config::{
    OllamaGenerateConfig, REQUIRED_PARAMS, SUPPORTED_OPENAI_PARAMS, complete_url, map_openai_params,
};
use betterlite_provider::ollama_generate::error::OllamaGenerateError;
use betterlite_provider::ollama_generate::reply::{ReplyInput, transform_response};
use betterlite_provider::ollama_generate::request::{RequestInput, transform_request};
use betterlite_provider::ollama_generate::stream::{
    GenerateStreamChunk, GenerateStreamStep, OllamaGenerateIterator,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{ParamCase, ReplyCase, RequestCase};
use crate::helpers::HelperError;
use crate::helpers::HelperResult;

const NOW: i64 = 1_700_000_000;

struct WordsCounter;

#[async_trait]
impl TokenCounter for WordsCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Option<u64> {
        None
    }

    async fn count_text(&self, text: &str) -> Option<u64> {
        let words = text.split_whitespace().count();
        let count = u64::try_from(words).unwrap_or(u64::MAX);
        Some(count)
    }
}

pub(super) fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}

pub(super) fn failure(error: &OllamaGenerateError) -> Value {
    json!({
        "ok": false,
        "status": error.status()
    })
}

pub(super) fn param_cases(
    config: &OllamaGenerateConfig,
    cases: &[ParamCase],
    urls: &[Option<String>],
) -> Value {
    let mapped: Vec<Value> = cases
        .iter()
        .map(|case| {
            let mut optional = case.optional.clone();
            match map_openai_params(&case.non_default, &mut optional, &case.model) {
                Ok(()) => json!({"ok": true, "options": optional}),
                Err(error) => failure(&error),
            }
        })
        .collect();
    let found_urls: Vec<String> = urls
        .iter()
        .map(|api_base| complete_url(api_base.as_deref()))
        .collect();
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "required": REQUIRED_PARAMS,
        "config": config.get_config(),
        "mapped": mapped,
        "urls": found_urls,
    })
}

pub(super) fn request_cases(
    custom_prompts: &BTreeMap<String, CustomPromptTemplate>,
    cases: &[RequestCase],
) -> Value {
    let bodies: Vec<Value> = cases
        .iter()
        .map(|case| {
            let input = RequestInput {
                model: &case.model,
                messages: &case.messages,
                optional_params: &case.options,
                params: &case.litellm_params,
                custom_prompts,
            };
            match transform_request(&input) {
                Ok(body) => json!({"ok": true, "body": body}),
                Err(error) => failure(&error),
            }
        })
        .collect();
    json!({"bodies": bodies})
}

pub(super) fn reply_cases(cases: &[ReplyCase]) -> HelperResult {
    let runtime = runtime()?;
    let counter = WordsCounter;
    let ids = RandomIds;
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = ReplyInput {
            model: &case.model,
            body: case.body.as_bytes(),
            request_data: &case.request_data,
            created: NOW,
        };
        found.push(
            match runtime.block_on(transform_response(&input, &counter, &ids)) {
                Ok(response) => json!({"ok": true, "reply": reply_value(&response)}),
                Err(error) => failure(&error),
            },
        );
    }
    Ok(json!({"replies": found}))
}

fn reply_value(response: &OllamaChatResponse) -> Value {
    let Some(choice) = response.choices.first() else {
        return json!({"missing_choice": true});
    };
    let message = &choice.message;
    json!({
        "model": response.model,
        "created": response.created,
        "finish_reason": choice.finish_reason,
        "role": message.role,
        "content": content_value(message.content.as_deref()),
        "reasoning_content": message.reasoning_content,
        "tool_calls": message.tool_calls.as_deref().map(calls_value),
        "usage": [
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
            response.usage.total_tokens
        ],
    })
}

fn content_value(content: Option<&str>) -> Value {
    content.map_or(Value::Null, |text| {
        serde_json::from_str::<Value>(text).unwrap_or_else(|_error| Value::String(text.to_owned()))
    })
}

fn calls_value(calls: &[ToolCall]) -> Value {
    let values: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "id": normalize_id(&call.id),
                "type": call.kind,
                "name": call.function.name,
                "arguments": normalize_arguments(&call.function.arguments),
            })
        })
        .collect();
    Value::Array(values)
}

fn normalize_arguments(arguments: &str) -> Value {
    serde_json::from_str::<Value>(arguments).map_or_else(
        |_error| Value::String(arguments.to_owned()),
        |value| Value::String(serde_json::to_string(&value).unwrap_or_default()),
    )
}

fn normalize_id(id: &str) -> String {
    for prefix in ["call_", "chatcmpl-"] {
        if let Some(rest) = id.strip_prefix(prefix)
            && Uuid::parse_str(rest).is_ok()
        {
            return format!("{prefix}<uuid>");
        }
    }
    if Uuid::parse_str(id).is_ok() {
        return String::from("<uuid>");
    }
    id.to_owned()
}

pub(super) fn stream_cases(lines: &[String]) -> Value {
    let mut iterator = OllamaGenerateIterator::default();
    let ids = RandomIds;
    let chunks: Vec<Value> = lines
        .iter()
        .map(
            |line| match iterator.chunk_parser(line.as_bytes(), NOW, &ids) {
                Ok(GenerateStreamStep::Done(done)) => json!({
                    "ok": true,
                    "done": true,
                    "text": done.text,
                    "is_finished": done.is_finished,
                    "finish_reason": done.finish_reason,
                    "usage": done.usage.map(usage_value),
                }),
                Ok(GenerateStreamStep::Chunk(chunk)) => {
                    json!({"ok": true, "chunk": chunk_value(&chunk)})
                }
                Err(error) => json!({"ok": false, "status": error.status()}),
            },
        )
        .collect();
    json!({"chunks": chunks})
}

fn chunk_value(chunk: &GenerateStreamChunk) -> Value {
    let choices: Vec<Value> = chunk
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": choice.delta.content,
                "reasoning_content": choice.delta.reasoning_content,
            })
        })
        .collect();
    json!({
        "id": normalize_id(&chunk.id),
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "usage": chunk.usage.map(usage_value),
        "choices": choices,
    })
}

fn usage_value(usage: Usage) -> Value {
    json!([
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens
    ])
}
