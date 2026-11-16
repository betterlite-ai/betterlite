use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_a2a::a2a_text::{
    A2aResponse, Field, RecordOr, Shape, extract_text_from_a2a_response,
};
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;
use serde::Deserialize;

use crate::ollama_chat::ids::IdSource;

use super::MAX_TEXT_DEPTH;
use super::error::{A2aError, ErrorDetail, error_message, get_error_class};
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

#[derive(Deserialize)]
struct ReplyEnvelope {
    id: Option<Shape>,
    #[serde(default)]
    error: Field<RecordOr<ErrorDetail>>,
    #[serde(flatten)]
    response: A2aResponse,
}

fn parse_reply(input: &ReplyInput<'_>) -> Result<ReplyEnvelope, A2aError> {
    serde_json::from_slice(input.body).map_err(|source| {
        let message = if source.is_data() {
            String::from("the A2A reply is not a JSON object")
        } else {
            format!("Failed to parse A2A response: {source}")
        };
        get_error_class(&message, input.status, input.headers.clone())
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
    let envelope = parse_reply(input)?;
    if let Field::Present(error) = &envelope.error {
        return Err(get_error_class(
            &error_message(error),
            input.status,
            input.headers.clone(),
        ));
    }
    let content =
        extract_text_from_a2a_response(&envelope.response, MAX_TEXT_DEPTH).map_err(|source| {
            get_error_class(
                &format!("the A2A reply text cannot be extracted: {source}"),
                input.status,
                input.headers.clone(),
            )
        })?;
    let id = match envelope.id {
        None | Some(Shape::Null) => ids.next_id(),
        Some(Shape::Text(id)) => id,
        Some(Shape::Flag(_) | Shape::Number(_) | Shape::List(_) | Shape::Object(_)) => {
            return Err(get_error_class(
                "the A2A reply id is not a string",
                input.status,
                input.headers.clone(),
            ));
        }
    };
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
