use betterlite_prompts::arguments::json::{Json, Object};

use crate::straiker::v3::messages::{conversation_messages, message_text};

// What the user first said: the first `user` message, never the system prompt an OpenAI chat body carries as `messages[0]`, else a Responses `input` string, else `prompt`.
#[must_use]
pub fn first_message_text(request_body: &Object) -> String {
    let first_user = conversation_messages(request_body)
        .into_iter()
        .find(|message| message.get("role").and_then(Json::as_str) == Some("user"));
    if let Some(message) = first_user {
        return message_text(message);
    }
    let input = request_body
        .get("input")
        .filter(|value| matches!(value, Json::Text(_)));
    input
        .or_else(|| request_body.get("prompt"))
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_owned()
}

mod tests;
