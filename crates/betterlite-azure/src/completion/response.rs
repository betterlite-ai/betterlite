use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_policy::reply::{ReplyError, to_chat_model_response};
use betterlite_openai::text_completion_transport::reply::CompletionReply;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("the completion reply cannot be read as JSON")]
    Serialize(#[from] serde_json::Error),
    #[error("the converted response is not a JSON object")]
    NotAnObject,
    #[error(transparent)]
    Reply(#[from] ReplyError),
}

// the ModelResponse the conversion mutates, as its own model_dump reads it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatReply {
    pub id: Json,
    pub object: Json,
    pub created: Json,
    pub model: Json,
    pub system_fingerprint: Json,
    pub choices: Vec<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Json>,
    #[serde(rename = "_hidden_params", default)]
    pub hidden_params: Extra,
    #[serde(flatten)]
    pub extra: Extra,
}

// ModelResponse() before the conversion: the fresh id and created time it starts with
fn fresh_target(identifier: &dyn Fn() -> Uuid, clock: &dyn Fn() -> i64) -> Extra {
    let mut target = Extra::new();
    target.insert("choices".to_owned(), Json::List(Vec::new()));
    target.insert("created".to_owned(), Json::Number(Number::from(clock())));
    target.insert(
        "id".to_owned(),
        Json::Text(format!("chatcmpl-{}", identifier())),
    );
    target.insert("model".to_owned(), Json::Null);
    target.insert(
        "object".to_owned(),
        Json::Text("chat.completion".to_owned()),
    );
    target.insert("system_fingerprint".to_owned(), Json::Null);
    target
}

// the async path keeps the SDK model_dump's raw usage and original response; the sync path normalizes first
pub fn project(
    reply: &CompletionReply,
    identifier: &dyn Fn() -> Uuid,
    clock: &dyn Fn() -> i64,
    keep_original: bool,
) -> Result<ChatReply, ProjectionError> {
    let response = serde_json::to_value(reply)?;
    let target_json = serde_json::to_value(fresh_target(identifier, clock))?;
    let object_target = target_json
        .as_object()
        .cloned()
        .ok_or(ProjectionError::NotAnObject)?;
    let converted = to_chat_model_response(
        Some(response.clone()),
        Some(object_target),
        identifier,
        clock,
    )?;
    let converted_json = serde_json::to_value(converted)?;
    let mut projected: ChatReply = serde_json::from_value(converted_json)?;
    if keep_original {
        let raw: Extra = serde_json::from_value(response)?;
        projected.usage = raw.get("usage").cloned();
        projected
            .hidden_params
            .insert("original_response".to_owned(), Json::Object(raw));
    }
    Ok(projected)
}
