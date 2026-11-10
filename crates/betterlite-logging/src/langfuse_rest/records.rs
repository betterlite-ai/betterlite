use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Prompt {
    Chat(ChatPrompt),
    Text(TextPrompt),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PromptMeta {
    pub name: String,
    pub version: i64,
    pub config: Json,
    pub labels: Vec<String>,
    pub tags: Vec<String>,
    #[serde(rename = "commitMessage", default)]
    pub commit_message: Option<String>,
    #[serde(rename = "resolutionGraph", default)]
    pub resolution_graph: Option<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ChatPrompt {
    pub prompt: Vec<PromptMessage>,
    #[serde(flatten)]
    pub meta: PromptMeta,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TextPrompt {
    pub prompt: String,
    #[serde(flatten)]
    pub meta: PromptMeta,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PromptMessage {
    Chat(ChatMessage),
    Placeholder(PlaceholderMessage),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(rename = "type", default)]
    pub kind: Option<ChatMessageType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum ChatMessageType {
    #[serde(rename = "chatmessage")]
    ChatMessage,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PlaceholderMessage {
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: Option<PlaceholderMessageType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum PlaceholderMessageType {
    #[serde(rename = "placeholder")]
    Placeholder,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Projects {
    pub data: Vec<Project>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub organization: Organization,
    pub metadata: IndexMap<String, Json>,
    #[serde(rename = "retentionDays", default)]
    pub retention_days: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
}
