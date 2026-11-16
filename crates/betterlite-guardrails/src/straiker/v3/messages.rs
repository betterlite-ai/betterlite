use betterlite_prompts::arguments::json::{Json, Object};

// Every text block of a message, so a turn that opens with an image or a document still seeds on what the user wrote.
#[must_use]
pub fn message_text(message: &Object) -> String {
    match message.get("content") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::List(blocks)) => blocks
            .iter()
            .filter_map(|block| block.as_object()?.get("text")?.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => {
            String::new()
        }
    }
}

// The conversation lives in `messages`, or in the Responses API's `input` when `messages` is empty.
#[must_use]
pub fn conversation_messages(request_body: &Object) -> Vec<&Object> {
    let source = request_body
        .get("messages")
        .filter(|messages| messages.is_truthy())
        .or_else(|| request_body.get("input"));
    match source {
        Some(Json::List(items)) => items.iter().filter_map(Json::as_object).collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Vec::new(),
    }
}

mod tests;
