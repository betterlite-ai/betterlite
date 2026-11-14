use std::error::Error as StdError;

use betterlite_anthropic::response_content::wire::{
    AnthropicCompletionResponse, AnthropicToolCall,
};
use betterlite_anthropic::response_content::{
    AnthropicContentReply, ResponseContentError, project_response,
};
use betterlite_anthropic::usage::{
    AnthropicUsage, ReasoningTokenCounter, UsageError, calculate_usage,
};
use betterlite_models::decisions::Json as AnswerJson;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenAiReply {
    pub response: ChatResponse,
    pub hidden_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeReply {
    pub id: String,
    pub model: String,
    pub choices: Vec<ClaudeChoice>,
    pub usage: AnthropicUsage,
    pub hidden_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeChoice {
    pub index: i64,
    pub finish_reason: String,
    pub message: ClaudeMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<AnthropicToolCall>>,
    pub thinking_blocks: Option<Vec<AnswerJson>>,
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Error)]
pub enum ResponseError<CounterFailure = std::convert::Infallible>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error("the provider answer could not be read: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Content(#[from] ResponseContentError),
    #[error(transparent)]
    Usage(#[from] UsageError<CounterFailure>),
}

pub struct ResponseInput<'request> {
    pub model: &'request str,
    pub raw_response: &'request str,
    pub request_data: &'request Json,
    pub messages: &'request [Json],
}

pub struct PostCall<'call> {
    pub input: &'call [Json],
    pub api_key: &'call str,
    pub complete_input_dict: &'call Json,
    pub original_response: &'call AnswerJson,
}

pub trait PostCallLogger {
    fn post_call(&self, call: PostCall<'_>);
}

pub fn transform_response_openai(
    input: &ResponseInput<'_>,
    logger: &dyn PostCallLogger,
) -> Result<OpenAiReply, ResponseError> {
    let body: AnswerJson = serde_json::from_str(input.raw_response).map_err(decode)?;
    logger.post_call(PostCall {
        input: input.messages,
        api_key: "",
        complete_input_dict: input.request_data,
        original_response: &body,
    });
    let mut response: ChatResponse = serde_json::from_str(input.raw_response).map_err(decode)?;
    response.model = format!("snowflake/{}", response.model);
    Ok(OpenAiReply {
        response,
        hidden_model: input.model.to_owned(),
    })
}

pub fn transform_response_claude<Counter>(
    input: &ResponseInput<'_>,
    logger: &dyn PostCallLogger,
    counter: &Counter,
) -> Result<ClaudeReply, ResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let body: AnswerJson = serde_json::from_str(input.raw_response).map_err(decode)?;
    logger.post_call(PostCall {
        input: input.messages,
        api_key: "",
        complete_input_dict: input.request_data,
        original_response: &body,
    });
    let wire: AnthropicCompletionResponse =
        serde_json::from_str(input.raw_response).map_err(decode)?;
    let reply = project_response(&wire, None)?;
    let usage_object = field(&body, "usage")
        .cloned()
        .unwrap_or_else(|| AnswerJson::Object(IndexMap::new()));
    let usage = calculate_usage(
        &usage_object,
        reply.reasoning_content.as_deref(),
        Some(&body),
        None,
        counter,
    )?;
    let response_model = field(&body, "model")
        .and_then(answer_text)
        .unwrap_or(input.model);
    Ok(ClaudeReply {
        id: field(&body, "id")
            .and_then(answer_text)
            .unwrap_or("")
            .to_owned(),
        model: format!("snowflake/{response_model}"),
        choices: vec![ClaudeChoice {
            index: 0,
            finish_reason: stop_reason(&body).to_owned(),
            message: claude_message(reply),
        }],
        usage,
        hidden_model: input.model.to_owned(),
    })
}

// LiteLLM builds exactly one assistant choice at index 0 and empties become None
fn claude_message(reply: AnthropicContentReply) -> ClaudeMessage {
    ClaudeMessage {
        role: "assistant".to_owned(),
        content: (!reply.content.is_empty()).then_some(reply.content),
        tool_calls: (!reply.tool_calls.is_empty()).then_some(reply.tool_calls),
        thinking_blocks: reply.thinking_blocks,
        reasoning_content: reply.reasoning_content,
    }
}

// A missing stop_reason reads as end_turn, so it maps to stop like every value outside the map
fn stop_reason(body: &AnswerJson) -> &'static str {
    match field(body, "stop_reason").and_then(answer_text) {
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        Some(_) | None => "stop",
    }
}

fn field<'answer>(answer: &'answer AnswerJson, name: &str) -> Option<&'answer AnswerJson> {
    if let AnswerJson::Object(fields) = answer {
        return fields.get(name);
    }
    None
}

fn answer_text(answer: &AnswerJson) -> Option<&str> {
    if let AnswerJson::Text(text) = answer {
        return Some(text.as_str());
    }
    None
}

fn decode<CounterFailure>(source: serde_json::Error) -> ResponseError<CounterFailure>
where
    CounterFailure: StdError + Send + Sync + 'static,
{
    ResponseError::Decode { source }
}

#[cfg(test)]
mod tests;
