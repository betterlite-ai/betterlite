use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_a2a::a2a_text::{A2aResponse, extract_text_from_a2a_response};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;

use crate::ollama_chat::ids::IdSource;

use super::MAX_TEXT_DEPTH;
use super::error::{A2aError, get_error_class};
use super::records::{A2aChoice, A2aReply, A2aReplyMessage, Usage};

// LiteLLM estimates the A2A usage with the gpt-3.5-turbo encoding
const TOKEN_MODEL: &str = "gpt-3.5-turbo";

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<u64, CountError>;
    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<u64, CountError>;
}

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub status: u16,
    pub headers: &'request BTreeMap<String, String>,
    pub body: &'request [u8],
    pub messages: &'request [Message],
}

fn reply_error_message(fields: &Object) -> String {
    let detail = fields
        .get("error")
        .and_then(Json::as_object)
        .and_then(|error| error.get("message"))
        .and_then(Json::as_str)
        .unwrap_or("Unknown error");
    format!("A2A error: {detail}")
}

fn parse_reply(input: &ReplyInput<'_>) -> Result<(Json, Object), A2aError> {
    let body: Json = serde_json::from_slice(input.body).map_err(|source| {
        get_error_class(
            &format!("Failed to parse A2A response: {source}"),
            input.status,
            input.headers.clone(),
        )
    })?;
    let Json::Object(fields) = &body else {
        return Err(get_error_class(
            "the A2A reply is not a JSON object",
            input.status,
            input.headers.clone(),
        ));
    };
    if fields.contains_key("error") {
        return Err(get_error_class(
            &reply_error_message(fields),
            input.status,
            input.headers.clone(),
        ));
    }
    let fields = fields.clone();
    Ok((body, fields))
}

fn reply_id(
    fields: &Object,
    input: &ReplyInput<'_>,
    ids: &dyn IdSource,
) -> Result<String, A2aError> {
    match fields.get("id") {
        None | Some(Json::Null) => Ok(ids.next_id()),
        Some(Json::Text(id)) => Ok(id.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(get_error_class(
                "the A2A reply id is not a string",
                input.status,
                input.headers.clone(),
            ))
        }
    }
}

fn reply_text(body: &Json, input: &ReplyInput<'_>) -> Result<String, A2aError> {
    let shape = serde_json::to_value(body).map_err(|source| {
        get_error_class(
            &format!("the A2A reply text cannot be extracted: {source}"),
            input.status,
            input.headers.clone(),
        )
    })?;
    let response: A2aResponse = serde_json::from_value(shape).map_err(|source| {
        get_error_class(
            &format!("the A2A reply text cannot be extracted: {source}"),
            input.status,
            input.headers.clone(),
        )
    })?;
    extract_text_from_a2a_response(&response, MAX_TEXT_DEPTH).map_err(|source| {
        get_error_class(
            &format!("the A2A reply text cannot be extracted: {source}"),
            input.status,
            input.headers.clone(),
        )
    })
}

async fn estimate_usage(
    text: &str,
    messages: &[Message],
    counter: &dyn TokenCounter,
) -> Option<Usage> {
    let prompt_tokens = counter.count_messages(TOKEN_MODEL, messages).await.ok()?;
    let completion_tokens = counter
        .count_response_tokens(TOKEN_MODEL, text)
        .await
        .ok()?;
    let total_tokens = prompt_tokens.checked_add(completion_tokens)?;
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

pub async fn transform_response(
    input: &ReplyInput<'_>,
    ids: &dyn IdSource,
    counter: &dyn TokenCounter,
) -> Result<A2aReply, A2aError> {
    let (body, fields) = parse_reply(input)?;
    let content = reply_text(&body, input)?;
    let id = reply_id(&fields, input, ids)?;
    let usage = estimate_usage(&content, input.messages, counter).await;
    Ok(A2aReply {
        id,
        model: input.model.to_owned(),
        choices: vec![A2aChoice {
            index: 0,
            finish_reason: "stop",
            message: A2aReplyMessage {
                role: "assistant",
                content,
            },
        }],
        usage,
    })
}
