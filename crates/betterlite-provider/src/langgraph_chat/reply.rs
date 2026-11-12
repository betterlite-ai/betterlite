use async_trait::async_trait;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;

use super::dumps::dumps;
use super::error::{LangGraphError, LangGraphFailure, get_error_class};
use super::records::{LangGraphRole, Usage};

// LangGraph reports no token usage, so LiteLLM estimates it with the gpt-3.5-turbo encoding
const TOKEN_MODEL: &str = "gpt-3.5-turbo";

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<u64, CountError>;
    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<u64, CountError>;
}

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub status: u16,
    pub body: &'request [u8],
    pub messages: &'request [Message],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangGraphChoice {
    pub index: u32,
    pub finish_reason: &'static str,
    pub role: LangGraphRole,
    // a reply message whose content is null gives a null content
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangGraphReply {
    pub model: String,
    pub choices: Vec<LangGraphChoice>,
    pub usage: Option<Usage>,
}

fn message_content(message: &Object) -> Result<Option<String>, LangGraphFailure> {
    match message.get("content") {
        None => Ok(Some(String::new())),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangGraphFailure::ContentType)
        }
    }
}

fn is_text(message: &Object, field: &str, expected: &str) -> bool {
    message
        .get(field)
        .is_some_and(|value| value.as_str() == Some(expected))
}

// a matched message ends the search, even when it has no content
pub fn extract_content(response: &Json) -> Result<Option<String>, LangGraphFailure> {
    let Json::Object(fields) = response else {
        return Err(LangGraphFailure::ReplyNotObject);
    };
    if let Some(Json::List(messages)) = fields.get("messages") {
        for message in messages.iter().rev() {
            if let Json::Object(entry) = message
                && (is_text(entry, "type", "ai") || is_text(entry, "role", "assistant"))
            {
                return message_content(entry);
            }
        }
    }
    if let Some(Json::Object(values)) = fields.get("values")
        && let Some(Json::List(messages)) = values.get("messages")
    {
        for message in messages.iter().rev() {
            if let Json::Object(entry) = message
                && is_text(entry, "type", "ai")
            {
                return message_content(entry);
            }
        }
    }
    tracing::warn!("Could not extract content from LangGraph response, returning raw");
    Ok(Some(dumps(response)?))
}

async fn estimate_usage(
    content: Option<&str>,
    messages: &[Message],
    counter: &dyn TokenCounter,
) -> Result<Usage, LangGraphFailure> {
    let prompt_tokens = counter
        .count_messages(TOKEN_MODEL, messages)
        .await
        .map_err(|source| LangGraphFailure::TokenCount { source })?;
    let text = content.ok_or(LangGraphFailure::NoReplyText)?;
    let completion_tokens = counter
        .count_response_tokens(TOKEN_MODEL, text)
        .await
        .map_err(|source| LangGraphFailure::TokenCount { source })?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(LangGraphFailure::UsageOverflow)?;
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

async fn build_reply(
    input: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
) -> Result<LangGraphReply, LangGraphFailure> {
    let response: Json =
        serde_json::from_slice(input.body).map_err(|source| LangGraphFailure::Decode { source })?;
    let content = extract_content(&response)?;
    let usage = match estimate_usage(content.as_deref(), input.messages, counter).await {
        Ok(usage) => Some(usage),
        Err(error) => {
            tracing::warn!("Failed to calculate token usage: {error}");
            None
        }
    };
    Ok(LangGraphReply {
        model: input.model.to_owned(),
        choices: vec![LangGraphChoice {
            index: 0,
            finish_reason: "stop",
            role: LangGraphRole::Assistant,
            content,
        }],
        usage,
    })
}

pub async fn transform_response(
    input: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
) -> Result<LangGraphReply, LangGraphError> {
    build_reply(input, counter).await.map_err(|failure| {
        tracing::error!("Error processing LangGraph response: {failure}");
        get_error_class(
            &format!("Error processing response: {failure}"),
            input.status,
        )
    })
}
