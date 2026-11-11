use async_trait::async_trait;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Content, Message, convert_content_list_to_str};
use betterlite_provider::ollama_chat::config::{
    OllamaChatConfig, SUPPORTED_OPENAI_PARAMS, complete_url, map_openai_params,
    validate_environment,
};
use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_provider::ollama_chat::reply::{
    ChatChoice, OllamaChatResponse, ReplyInput, transform_response,
};
use betterlite_provider::ollama_chat::request::{RequestInput, transform_request};
use betterlite_provider::ollama_chat::stream::chunk::OllamaStreamChunk;
use betterlite_provider::ollama_chat::stream::{OllamaChatIterator, OllamaStreamStep};
use betterlite_provider::ollama_chat::usage::TokenCounter;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::script::{ParamCase, ReplyCase, RequestCase, ValidateCase, runtime};
use crate::helpers::HelperResult;

const NOW: i64 = 1_700_000_000;

struct WordsCounter;

#[async_trait]
impl TokenCounter for WordsCounter {
    async fn count_messages(&self, messages: &[Message]) -> Option<u64> {
        let words = messages.iter().map(|message| {
            convert_content_list_to_str(message)
                .split_whitespace()
                .count()
        });
        Some(words.fold(0_u64, |total, count| {
            total.saturating_add(u64::try_from(count).unwrap_or(u64::MAX))
        }))
    }

    async fn count_text(&self, text: &str) -> Option<u64> {
        Some(u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX))
    }
}

fn user_messages() -> Vec<Message> {
    vec![Message {
        role: String::from("user"),
        content: Some(Content::Text(String::from("hi"))),
        name: None,
        provider_specific_fields: Object::new(),
    }]
}

fn normalize_id(id: &str) -> String {
    match id.strip_prefix("call_") {
        Some(rest) if Uuid::parse_str(rest).is_ok() => String::from("call_<uuid>"),
        _ if Uuid::parse_str(id).is_ok() => String::from("<uuid>"),
        _ => id.to_owned(),
    }
}

fn normalize_arguments(arguments: &str) -> String {
    serde_json::from_str::<Value>(arguments).map_or_else(
        |_error| arguments.to_owned(),
        |value| serde_json::to_string(&value).unwrap_or_default(),
    )
}

#[derive(Serialize)]
struct CallValue<'call> {
    id: Option<String>,
    #[serde(rename = "type")]
    kind: &'call str,
    name: Option<&'call str>,
    arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<u32>,
}

fn calls_json<'call, Calls>(calls: Calls) -> Value
where
    Calls: Iterator<
        Item = (
            Option<String>,
            &'call str,
            Option<&'call str>,
            String,
            Option<u32>,
        ),
    >,
{
    let values: Vec<CallValue<'call>> = calls
        .map(|(id, kind, name, arguments, index)| CallValue {
            id,
            kind,
            name,
            arguments,
            index,
        })
        .collect();
    serde_json::to_value(values).unwrap_or_default()
}

fn reply_value(response: &OllamaChatResponse) -> Value {
    response.choices.first().map_or_else(
        || json!({"missing_choice": true}),
        |choice| reply_choice(response, choice),
    )
}

fn reply_choice(response: &OllamaChatResponse, choice: &ChatChoice) -> Value {
    json!({
        "model": response.model,
        "created": response.created,
        "finish_reason": choice.finish_reason,
        "role": choice.message.role,
        "content": choice.message.content,
        "reasoning_content": choice.message.reasoning_content,
        "tool_calls": choice.message.tool_calls.as_deref().map(|calls| calls_json(
            calls.iter().map(|call| (
                Some(normalize_id(&call.id)),
                call.kind.as_str(),
                call.function.name.as_deref(),
                normalize_arguments(&call.function.arguments),
                None,
            )),
        )),
        "usage": [response.usage.prompt_tokens, response.usage.completion_tokens, response.usage.total_tokens],
    })
}

fn chunk_value(chunk: &OllamaStreamChunk) -> Value {
    let choices: Vec<Value> = chunk
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": choice.delta.content,
                "reasoning_content": choice.delta.reasoning_content,
                "tool_calls": choice.delta.tool_calls.as_deref().map(|calls| calls_json(
                    calls.iter().map(|call| (
                        call.id.as_deref().map(normalize_id),
                        call.kind.as_str(),
                        call.function.name.as_deref(),
                        normalize_arguments(&call.function.arguments),
                        Some(call.index),
                    )),
                )),
            })
        })
        .collect();
    let usage = chunk.usage.map(|usage| {
        json!([
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        ])
    });
    json!({
        "id": normalize_id(&chunk.id),
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "usage": usage,
        "choices": choices,
    })
}

pub(super) fn params(
    config: &OllamaChatConfig,
    cases: &[ParamCase],
    validate: &[ValidateCase],
    urls: &[Option<String>],
) -> Value {
    let mapped: Vec<Value> = cases
        .iter()
        .map(|case| {
            let mut optional = case.optional.clone();
            match map_openai_params(&case.non_default, &mut optional, &case.model) {
                Ok(()) => json!({"ok": true, "options": optional}),
                Err(error) => json!({"ok": false, "status": error.status()}),
            }
        })
        .collect();
    let validated: Vec<Value> = validate
        .iter()
        .map(|case| {
            let mut headers = case.headers.clone();
            let key = case.api_key.as_deref().map(SecretString::from);
            validate_environment(&mut headers, key.as_ref());
            json!(headers)
        })
        .collect();
    let found: Vec<String> = urls
        .iter()
        .map(|api_base| complete_url(api_base.as_deref()))
        .collect();
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "config": config.get_config(),
        "mapped": mapped,
        "validated": validated,
        "urls": found,
    })
}

pub(super) fn requests(cases: &[RequestCase]) -> Value {
    let bodies: Vec<Value> = cases
        .iter()
        .map(|case| {
            let input = RequestInput {
                model: &case.model,
                messages: &case.messages,
                optional_params: &case.options,
            };
            match transform_request(&input, &case.config) {
                Ok(request) => json!({
                    "ok": true,
                    "body": &request,
                    "function_name": request.function_name,
                }),
                Err(error) => json!({"ok": false, "status": error.status()}),
            }
        })
        .collect();
    json!({"bodies": bodies})
}

pub(super) fn replies(cases: &[ReplyCase]) -> HelperResult {
    let runtime = runtime()?;
    let messages = user_messages();
    let counter = WordsCounter;
    let ids = RandomIds;
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = ReplyInput {
            model: &case.model,
            body: case.body.as_bytes(),
            request_data: &case.request_data,
            litellm_params: &case.litellm_params,
            messages: &messages,
            created: NOW,
        };
        found.push(
            match runtime.block_on(transform_response(&input, &counter, &ids)) {
                Ok(response) => json!({"ok": true, "reply": reply_value(&response)}),
                Err(error) => json!({"ok": false, "status": error.status()}),
            },
        );
    }
    Ok(json!({"replies": found}))
}

pub(super) fn stream(lines: &[String]) -> Value {
    let mut iterator = OllamaChatIterator::default();
    let ids = RandomIds;
    let chunks: Vec<Value> = lines
        .iter()
        .map(|line| match iterator.next_chunk(line, NOW, &ids) {
            Ok(OllamaStreamStep::Chunk(chunk)) => json!({"ok": true, "chunk": chunk_value(&chunk)}),
            Ok(OllamaStreamStep::Skipped) => json!({"ok": true, "skipped": true}),
            Ok(OllamaStreamStep::Done) => json!({"ok": true, "done": true}),
            Err(error) => json!({"ok": false, "status": error.status()}),
        })
        .collect();
    json!({"chunks": chunks})
}
