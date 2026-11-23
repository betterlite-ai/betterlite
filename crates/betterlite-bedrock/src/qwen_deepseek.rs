pub mod error;
pub mod options;
pub mod prompt;
pub mod reply;

mod tests;

use std::convert::Infallible;

use betterlite_anthropic::usage::ReasoningTokenCounter as AnthropicReasoningCounter;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_models::decisions::Json;
use betterlite_prompts::reasoning_fields::content_split::parse_content_for_reasoning;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::converse_usage::transform::ReasoningTokenCounter as ConverseReasoningCounter;
use crate::invoke_response::{InvokeReply, InvokeReplyCall, RawInvokeReply, decode_invoke_reply};
use crate::pegasus::counter::TokenCounter;

use error::DeepseekReplyError as DeepseekError;
use error::QwenReplyError;
use prompt::{ASSISTANT_START, IM_END};
use reply::{Reply, Usage};

// The reply object fields every Qwen transform reads its generation and usage from.
type Fields = IndexMap<String, Json>;

// AmazonQwen2Config only changes the reply projection; its request and option code is Qwen3's.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QwenProjection {
    Qwen2,
    Qwen3,
}

// The produced text, wrapper removal and usage replacement shared by both Qwen reply transforms.
pub fn decode_qwen_reply(
    projection: QwenProjection,
    body: &[u8],
    mut response: Reply,
) -> Result<Reply, QwenReplyError> {
    let data: Json = serde_json::from_slice(body).map_err(QwenReplyError::Decode)?;
    let Json::Object(fields) = &data else {
        return Err(QwenReplyError::NotObject);
    };
    let generated = generation(fields, projection)?;
    let started = generated
        .strip_prefix(ASSISTANT_START)
        .unwrap_or(&generated);
    let generated = started.strip_suffix(IM_END).unwrap_or(started).to_owned();
    if let Some(choice) = response.choices.first_mut() {
        choice.message.content = Some(generated);
        choice.finish_reason = Some(String::from("stop"));
    }
    if let Some(usage) = fields.get("usage") {
        response.usage = Some(usage_from(usage)?);
    }
    Ok(response)
}

// Qwen2 prefers a truthy generation value and only then falls back to text; Qwen3 reads generation alone.
fn generation(fields: &Fields, projection: QwenProjection) -> Result<String, QwenReplyError> {
    if projection == QwenProjection::Qwen2 {
        if let Some(generation) = fields.get("generation").filter(|value| is_present(value)) {
            return text_value(generation, "generation");
        }
        return text_field(fields, "text");
    }
    text_field(fields, "generation")
}

// A missing generation key is empty; a present null or non-string is the source's AttributeError.
fn text_field(fields: &Fields, name: &'static str) -> Result<String, QwenReplyError> {
    let Some(value) = fields.get(name) else {
        return Ok(String::new());
    };
    text_value(value, name)
}

fn text_value(value: &Json, field: &'static str) -> Result<String, QwenReplyError> {
    if let Json::Text(text) = value {
        return Ok(text.clone());
    }
    Err(QwenReplyError::NotText { field })
}

fn usage_from(value: &Json) -> Result<Usage, QwenReplyError> {
    let Json::Object(fields) = value else {
        return Err(QwenReplyError::UsageNotObject);
    };
    Ok(Usage {
        prompt_tokens: usage_count(fields.get("prompt_tokens"), "prompt_tokens")?,
        completion_tokens: usage_count(fields.get("completion_tokens"), "completion_tokens")?,
        total_tokens: usage_count(fields.get("total_tokens"), "total_tokens")?,
    })
}

fn usage_count(value: Option<&Json>, name: &'static str) -> Result<i64, QwenReplyError> {
    match value {
        None => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(QwenReplyError::UsageCount { name }),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(QwenReplyError::UsageCount { name })
        }
    }
}

// The Qwen transforms log the source messages and response before re-raising, and nothing on success.
pub fn transform_qwen_response(
    projection: QwenProjection,
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: Reply,
) -> Result<Reply, QwenReplyError> {
    match decode_qwen_reply(projection, raw.body, response) {
        Ok(reply) => Ok(reply),
        Err(error) => {
            log_qwen_failure(call, raw, &error);
            Err(error)
        }
    }
}

fn log_qwen_failure(
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    error: &QwenReplyError,
) {
    let InvokeReplyCall { messages, log, .. } = call;
    let mut passthrough = IndexMap::new();
    passthrough.insert(String::from("error"), Json::Text(error.to_string()));
    post_call(
        log.state,
        &PostCall {
            original_response: Json::Text(response_text(raw.body)),
            input: messages.clone(),
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: None,
                request_str: None,
                openai_sdk: None,
                passthrough,
            },
        },
        log.env,
    );
}

// A body that is not UTF-8 is named by its byte count, the way the Converse reply log names it.
fn response_text(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_source| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

// AmazonDeepSeekR1Config.transform_response: the Llama reply first, then the optional reasoning split.
pub async fn decode_deepseek_reply<ChatCounter, Reasoning>(
    call: InvokeReplyCall<'_, '_, '_>,
    raw: &RawInvokeReply<'_>,
    response: Reply,
    counter: &ChatCounter,
    reasoning: &Reasoning,
) -> Result<Reply, DeepseekError>
where
    ChatCounter: TokenCounter + Sync,
    Reasoning: Sync
        + AnthropicReasoningCounter<Failure = Infallible>
        + ConverseReasoningCounter<Failure = Infallible>,
{
    let request_data = call.request_data;
    let decoded = decode_invoke_reply(
        call,
        raw,
        reply::chat_response(&response),
        counter,
        reasoning,
    )
    .await?;
    let InvokeReply::Chat(chat) = decoded else {
        return Err(DeepseekError::NotChat);
    };
    let mut reply = reply::from_chat(chat, &response)?;
    apply_reasoning(request_data, &mut reply)?;
    Ok(reply)
}

// Only a prompt that trims to a trailing <think>, and a first content, reach the reasoning split.
fn apply_reasoning(
    request_data: &IndexMap<String, Json>,
    reply: &mut Reply,
) -> Result<(), DeepseekError> {
    let Some(prompt) = request_data.get("prompt") else {
        return Ok(());
    };
    if !is_present(prompt) {
        return Ok(());
    }
    let Json::Text(prompt) = prompt else {
        return Err(DeepseekError::PromptNotText);
    };
    if !strip_str_whitespace(prompt).ends_with("<think>") {
        return Ok(());
    }
    let Some(choice) = reply.choices.first_mut() else {
        return Ok(());
    };
    let Some(content) = choice
        .message
        .content
        .clone()
        .filter(|content| !content.is_empty())
    else {
        return Ok(());
    };
    let prefixed = format!("<think>{content}");
    let (parsed_reasoning, parsed_content) = parse_content_for_reasoning(Some(&prefixed));
    choice.message.content = parsed_content;
    let mut fields = choice
        .message
        .provider_specific_fields
        .take()
        .unwrap_or_default();
    if let Some(reasoning) = parsed_reasoning.filter(|reasoning| !reasoning.is_empty()) {
        fields.insert(String::from("reasoning_content"), Json::Text(reasoning));
    }
    choice.message.provider_specific_fields = (!fields.is_empty()).then_some(fields);
    Ok(())
}

// Python's str.strip() also strips the four ASCII separators Rust's whitespace class leaves alone.
fn strip_str_whitespace(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
    })
}

// Python truthiness for the request values that gate the reasoning split.
fn is_present(value: &Json) -> bool {
    match value {
        Json::Null | Json::Bool(false) => false,
        Json::Bool(true) => true,
        Json::Number(number) => number.as_f64().is_some_and(|parsed| parsed != 0.0_f64),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
