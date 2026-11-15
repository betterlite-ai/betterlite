use std::collections::BTreeMap;

use serde_json::Value;

use crate::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use crate::sagemaker_completion::error::SagemakerError;

pub trait TextTokenCounter {
    fn count_response_tokens(&self, text: &str) -> u64;
}

pub struct ReplyInput<'request> {
    pub reply: &'request Value,
    pub model: &'request str,
    pub prompt: &'request str,
    pub id: String,
    pub created: u64,
}

pub fn transform_response(
    input: &ReplyInput<'_>,
    counter: &dyn TextTokenCounter,
) -> Result<ChatResponse, SagemakerError> {
    let text = reply_text(input.reply)?;
    let content = strip_echo(text, input.prompt);
    let prompt_tokens = counter.count_response_tokens(input.prompt);
    let completion_tokens = counter.count_response_tokens(content);
    Ok(ChatResponse {
        id: input.id.clone(),
        object: String::from("chat.completion"),
        created: input.created,
        model: input.model.to_owned(),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: Some(content.to_owned()),
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: Some(String::from("stop")),
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        system_fingerprint: None,
        service_tier: None,
    })
}

fn reply_text(reply: &Value) -> Result<&str, SagemakerError> {
    let item = match reply {
        Value::Array(items) => items.first().ok_or_else(|| parse_error(reply))?,
        Value::Object(_) => reply,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err(parse_error(reply));
        }
    };
    let Value::Object(fields) = item else {
        return Err(parse_error(reply));
    };
    match fields
        .get("generation")
        .or_else(|| fields.get("generated_text"))
    {
        Some(Value::String(text)) => Ok(text),
        Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_),
        ) => Err(parse_error(reply)),
        None => Ok(""),
    }
}

fn strip_echo<'text>(text: &'text str, prompt: &str) -> &'text str {
    // the pinned transform strips the echoed prompt only for a template that opens with the bos token
    if prompt.contains("<s>") {
        return text.strip_prefix(prompt).unwrap_or(text);
    }
    text
}

fn parse_error(reply: &Value) -> SagemakerError {
    SagemakerError {
        status_code: 500,
        message: format!(
            "LiteLLM Error: Unable to parse sagemaker RAW RESPONSE {}",
            serde_json::to_string(reply).unwrap_or_default()
        ),
        headers: None,
    }
}

mod tests;
