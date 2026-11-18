use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use indexmap::IndexMap;

pub(super) const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

pub(super) const fn text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(found) => Some(found.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// the message dict LiteLLM's prefix walk sees, rebuilt from the typed request message
#[must_use]
pub fn message_json(message: &Message) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("role".to_owned(), Json::Text(message.role.clone()));
    if let Some(content) = &message.content {
        fields.insert("content".to_owned(), content_json(content));
    }
    if let Some(name) = &message.name {
        fields.insert("name".to_owned(), Json::Text(name.clone()));
    }
    for (key, value) in &message.provider_specific_fields {
        fields.insert(key.clone(), from_prompt(value));
    }
    Json::Object(fields)
}

#[must_use]
pub fn messages_json(messages: &[Message]) -> Vec<Json> {
    let mut converted = Vec::with_capacity(messages.len());
    for message in messages {
        converted.push(message_json(message));
    }
    converted
}

fn content_json(content: &Content) -> Json {
    match content {
        Content::Text(text) => Json::Text(text.clone()),
        Content::Parts(parts) => Json::List(parts.iter().map(part_json).collect()),
    }
}

fn part_json(part: &ContentPart) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("type".to_owned(), Json::Text(part.kind.clone()));
    for (key, value) in &part.fields {
        fields.insert(key.clone(), from_prompt(value));
    }
    Json::Object(fields)
}

fn from_prompt(value: &PromptJson) -> Json {
    match value {
        PromptJson::Null => Json::Null,
        PromptJson::Bool(flag) => Json::Bool(*flag),
        PromptJson::Number(number) => Json::Number(number.clone()),
        PromptJson::Text(text) => Json::Text(text.clone()),
        PromptJson::List(items) => Json::List(items.iter().map(from_prompt).collect()),
        PromptJson::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), from_prompt(item)))
                .collect(),
        ),
    }
}
