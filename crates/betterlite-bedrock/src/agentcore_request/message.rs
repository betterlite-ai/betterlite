use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as Field, Object};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use indexmap::IndexMap;
use serde::Deserialize;

// LiteLLM reads a content block's type as block.get("type"), so an absent type is a real state here
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentCoreMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<AgentCoreContent>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub provider_specific_fields: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum AgentCoreContent {
    Text(String),
    Parts(Vec<AgentCoreBlock>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AgentCoreBlock {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(flatten)]
    pub fields: Object,
}

// B218 reads only each block's text field, so a block without a type is the same text
pub(super) fn as_prompt_message(message: &AgentCoreMessage) -> Message {
    Message {
        role: message.role.clone(),
        content: message.content.as_ref().map(|content| match content {
            AgentCoreContent::Text(text) => Content::Text(text.clone()),
            AgentCoreContent::Parts(parts) => Content::Parts(
                parts
                    .iter()
                    .map(|part| ContentPart {
                        kind: part.kind.clone().unwrap_or_default(),
                        fields: part.fields.clone(),
                    })
                    .collect(),
            ),
        }),
        name: message.name.clone(),
        provider_specific_fields: message.provider_specific_fields.clone(),
    }
}

pub(super) fn forwarded_block(block: &AgentCoreBlock) -> Json {
    let mut fields = IndexMap::new();
    if let Some(kind) = &block.kind {
        fields.insert("type".to_owned(), Json::Text(kind.clone()));
    }
    fields.extend(
        block
            .fields
            .iter()
            .map(|(name, entry)| (name.clone(), decision_json(entry))),
    );
    Json::Object(fields)
}

fn decision_json(field: &Field) -> Json {
    match field {
        Field::Null => Json::Null,
        Field::Bool(flag) => Json::Bool(*flag),
        Field::Number(number) => Json::Number(number.clone()),
        Field::Text(text) => Json::Text(text.clone()),
        Field::List(items) => Json::List(items.iter().map(decision_json).collect()),
        Field::Object(entries) => Json::Object(
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), decision_json(entry)))
                .collect(),
        ),
    }
}
