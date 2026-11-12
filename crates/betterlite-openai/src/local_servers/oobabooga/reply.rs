use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use super::error::{OobaboogaError, ReplyError};

// absent and null differ here: LiteLLM tests `"error" in reply` and assigns a null content
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Field<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null,
    Present(T),
}

#[derive(Debug, Deserialize)]
struct ChatBody {
    #[serde(default)]
    error: Field<Value>,
    choices: Option<Vec<Choice>>,
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Field<String>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ChatReply {
    pub model: String,
    pub created: i64,
    pub content: Option<String>,
    pub usage: ChatUsage,
}

#[derive(Debug, Deserialize)]
struct EmbeddingBody {
    #[serde(default)]
    error: Field<Value>,
    status_code: Option<u16>,
    data: Option<Vec<EmbeddingItem>>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingItem {
    embedding: Option<Vec<Number>>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct EmbeddingReply {
    pub model: String,
    pub embedding: Vec<Number>,
    pub prompt_tokens: usize,
    pub total_tokens: usize,
}

// LiteLLM puts the server's error value into the message as is, a string stays a string
fn error_message(error: Field<Value>) -> Option<String> {
    match error {
        Field::Absent => None,
        Field::Null => Some(Value::Null.to_string()),
        Field::Present(Value::String(text)) => Some(text),
        Field::Present(other) => Some(other.to_string()),
    }
}

pub fn transform_response(
    model: &str,
    status: u16,
    raw_text: &str,
    created: i64,
) -> Result<ChatReply, ReplyError> {
    let Ok(value) = serde_json::from_str::<Value>(raw_text) else {
        return Err(ReplyError::Server(OobaboogaError {
            status_code: status,
            message: raw_text.to_owned(),
            headers: None,
        }));
    };
    let body = ChatBody::deserialize(value).map_err(|source| ReplyError::Shape { source })?;
    let failure = |message: &str| OobaboogaError {
        status_code: status,
        message: message.to_owned(),
        headers: None,
    };
    if let Some(message) = error_message(body.error) {
        return Err(ReplyError::Server(failure(&message)));
    }
    let choices = body.choices.ok_or_else(|| failure("'choices'"))?;
    let first = choices
        .into_iter()
        .next()
        .ok_or_else(|| failure("list index out of range"))?;
    let message = first.message.ok_or_else(|| failure("'message'"))?;
    let content = match message.content {
        Field::Absent => return Err(ReplyError::Server(failure("'content'"))),
        Field::Null => None,
        Field::Present(text) => Some(text),
    };
    // LiteLLM reads usage last, so a bad usage never hides a missing choice
    let raw_usage = body.usage.ok_or(ReplyError::Missing { field: "usage" })?;
    let usage = ChatUsage::deserialize(raw_usage).map_err(|source| ReplyError::Shape { source })?;
    Ok(ChatReply {
        model: model.to_owned(),
        created,
        content,
        usage,
    })
}

pub fn transform_embedding_response(
    model: &str,
    raw_text: &str,
) -> Result<EmbeddingReply, ReplyError> {
    let body: EmbeddingBody =
        serde_json::from_str(raw_text).map_err(|source| ReplyError::Shape { source })?;
    if let Some(message) = error_message(body.error) {
        return Err(ReplyError::Server(OobaboogaError {
            status_code: body.status_code.unwrap_or(500),
            message,
            headers: None,
        }));
    }
    let first = body
        .data
        .and_then(|data| data.into_iter().next())
        .ok_or(ReplyError::Missing { field: "data[0]" })?;
    let embedding = first
        .embedding
        .ok_or(ReplyError::Missing { field: "embedding" })?;
    let tokens = embedding.len();
    Ok(EmbeddingReply {
        model: model.to_owned(),
        embedding,
        prompt_tokens: tokens,
        total_tokens: tokens,
    })
}
