use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::SapModelError;
use super::content::MessageContent;
use super::presence::Given;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TextKind {
    Text,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    ImageUrl,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FunctionKind {
    Function,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemRole {
    System,
    Developer,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    User,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AssistantRole {
    Assistant,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolRole {
    Tool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextContent {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<TextKind>,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageURLContent {
    pub url: String,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub detail: Given<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageContent {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<ImageKind>,
    pub image_url: ImageURLContent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionObj {
    pub name: String,
    pub arguments: String,
}

// FunctionTool.ensure_object_type, applied before the dict type check.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(try_from = "Json")]
pub struct Parameters(Json);

impl Parameters {
    fn normalized(object: Object) -> Self {
        let mut normalized = Object::new();
        normalized.insert(String::from("type"), Json::Text(String::from("object")));
        normalized.extend(object);
        normalized
            .entry(String::from("properties"))
            .or_insert_with(|| Json::Object(Object::new()));
        Self(Json::Object(normalized))
    }
}

impl TryFrom<Json> for Parameters {
    type Error = SapModelError;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        match value {
            Json::Null => Ok(Self::normalized(Object::new())),
            Json::Object(object) => Ok(Self::normalized(object)),
            Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                Err(SapModelError::Rejected {
                    message: String::from("parameters must be an object"),
                })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FunctionTool {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub description: Given<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub parameters: Given<Parameters>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub strict: Given<bool>,
}

impl FunctionTool {
    fn full(&self) -> Self {
        Self {
            description: self.description.full(String::new()),
            name: self.name.clone(),
            parameters: self.parameters.full(Parameters::normalized(Object::new())),
            strict: self.strict.full(false),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatCompletionTool {
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<FunctionKind>,
    pub function: FunctionTool,
}

impl ChatCompletionTool {
    pub(crate) fn full(&self) -> Self {
        Self {
            type_: self.type_.full(FunctionKind::Function),
            function: self.function.full(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MessageToolCall {
    pub id: String,
    #[serde(rename = "type", default, skip_serializing_if = "Given::is_missing")]
    pub type_: Given<FunctionKind>,
    pub function: FunctionObj,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SAPMessage {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub role: Given<SystemRole>,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum UserPart {
    Text(TextContent),
    Image(ImageContent),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    TextPart(TextContent),
    ImagePart(ImageContent),
    Parts(Vec<UserPart>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SAPUserMessage {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub role: Given<UserRole>,
    pub content: UserContent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SAPAssistantMessage {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub role: Given<AssistantRole>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub content: Given<MessageContent>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub refusal: Given<String>,
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub tool_calls: Given<Vec<MessageToolCall>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SAPToolChatMessage {
    #[serde(default, skip_serializing_if = "Given::is_missing")]
    pub role: Given<ToolRole>,
    pub tool_call_id: String,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChatMessage {
    System(SAPMessage),
    User(SAPUserMessage),
    Assistant(SAPAssistantMessage),
    Tool(SAPToolChatMessage),
}
