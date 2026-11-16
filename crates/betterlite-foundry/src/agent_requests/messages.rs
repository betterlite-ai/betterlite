use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{
    Content, ContentPart, Message, convert_content_list_to_str,
};
use serde::Serialize;

use super::error::AgentRequestError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConvertedMessage {
    pub role: Json,
    pub content: String,
}

// transform_request keeps only the role and the text of each message; LiteLLM discards every other key here
pub fn convert_messages(messages: &[Object]) -> Result<Vec<ConvertedMessage>, AgentRequestError> {
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        converted.push(convert_message(message)?);
    }
    Ok(converted)
}

fn convert_message(message: &Object) -> Result<ConvertedMessage, AgentRequestError> {
    let role = message
        .get("role")
        .cloned()
        .unwrap_or_else(|| Json::Text(String::from("user")));
    let content = content_text(message)?;
    Ok(ConvertedMessage { role, content })
}

fn content_text(message: &Object) -> Result<String, AgentRequestError> {
    match message.get("content") {
        None => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::List(items)) => text_from_list(message, items),
        Some(Json::Null) => Ok(String::from("None")),
        Some(Json::Bool(flag)) => Ok(String::from(if *flag { "True" } else { "False" })),
        Some(Json::Number(number)) => Ok(number.to_string()),
        Some(Json::Object(_)) => Err(AgentRequestError::FieldType { field: "content" }),
    }
}

// a content list goes through the shared converter, which also appends the message's search results
fn text_from_list(message: &Object, items: &[Json]) -> Result<String, AgentRequestError> {
    let mut parts = Vec::with_capacity(items.len());
    for item in items {
        parts.push(content_part(item)?);
    }
    let converted = Message {
        role: String::from("user"),
        content: Some(Content::Parts(parts)),
        name: None,
        provider_specific_fields: search_results(message),
    };
    Ok(convert_content_list_to_str(&converted))
}

fn content_part(item: &Json) -> Result<ContentPart, AgentRequestError> {
    let Json::Object(fields) = item else {
        return Err(AgentRequestError::FieldType { field: "content" });
    };
    let mut remaining = fields.clone();
    let kind = remaining
        .shift_remove("type")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    Ok(ContentPart {
        kind,
        fields: remaining,
    })
}

fn search_results(message: &Object) -> Object {
    let mut fields = Object::new();
    if let Some(value) = message.get("search_results") {
        fields.insert(String::from("search_results"), value.clone());
    }
    fields
}
