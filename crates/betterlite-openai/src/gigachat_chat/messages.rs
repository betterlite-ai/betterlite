use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use serde_with::skip_serializing_none;

use crate::chat::{ChatMessage, ContentPart, MessageContent, Role, ToolCall};
use crate::gigachat_chat::config::GigachatImageUploader;
use crate::gigachat_chat::error::GigaChatError;
use crate::openai_base_config::Object;

#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GigachatMessage {
    role: Role,
    content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<String>,
    function_call: Option<GigachatFunctionCall>,
    tool_calls: Option<Vec<ToolCall>>,
    tool_call_id: Option<String>,
    #[serde(flatten)]
    passthrough: IndexMap<String, Value>,
}

impl GigachatMessage {
    fn drop_replaced_fields(&mut self) {
        if self.function_call.is_some() {
            self.passthrough.shift_remove("function_call");
        }
        if !self.attachments.is_empty() {
            self.passthrough.shift_remove("attachments");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GigachatFunctionCall {
    name: String,
    arguments: Value,
}

#[derive(Deserialize)]
struct Part {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
    image_url: Option<ImageUrl>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ImageUrl {
    Link(String),
    Fields { url: Option<String> },
}

pub async fn transform_messages(
    messages: &[ChatMessage],
    uploader: &dyn GigachatImageUploader,
) -> Result<Vec<GigachatMessage>, GigaChatError> {
    let mut transformed = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let role = match message.role {
            Role::Developer => Role::System,
            Role::System if index > 0 => Role::User,
            Role::Tool => Role::Function,
            Role::System | Role::User | Role::Assistant | Role::Function => message.role,
        };
        let (content, attachments) = match (&message.content, message.role) {
            (content, Role::Tool) => (tool_content(content.as_ref()), Vec::new()),
            (None, _) => (String::new(), Vec::new()),
            (Some(MessageContent::Text(text)), _) => (text.clone(), Vec::new()),
            (Some(MessageContent::Parts(parts)), _) => join_parts(parts, uploader).await?,
        };
        let first_call = message.tool_calls.as_ref().and_then(|calls| calls.first());
        let mut built = GigachatMessage {
            role,
            content,
            attachments,
            function_call: first_call.map(|call| GigachatFunctionCall {
                name: call.function.name.clone(),
                arguments: serde_json::from_str(&call.function.arguments)
                    .unwrap_or_else(|_| Value::Object(Object::new())),
            }),
            tool_calls: message.tool_calls.clone().filter(|_| first_call.is_none()),
            tool_call_id: message.tool_call_id.clone(),
            passthrough: message.passthrough.clone(),
        };
        built.drop_replaced_fields();
        transformed.push(built);
    }
    Ok(transformed)
}

fn tool_content(content: Option<&MessageContent>) -> String {
    match content {
        Some(MessageContent::Parts(parts)) => json!(parts).to_string(),
        Some(MessageContent::Text(text)) if serde_json::from_str::<Value>(text).is_ok() => {
            text.clone()
        }
        Some(MessageContent::Text(text)) => json!(text).to_string(),
        None => String::from("\"\""),
    }
}

async fn join_parts(
    parts: &[ContentPart],
    uploader: &dyn GigachatImageUploader,
) -> Result<(String, Vec<String>), GigaChatError> {
    let mut texts = Vec::new();
    let mut attachments = Vec::new();
    for entry in parts {
        let part: Part = serde_json::from_value(json!(entry)).map_err(|source| GigaChatError {
            status_code: 400,
            message: format!("a content part holds declared fields: {source}"),
            headers: None,
            source: Some(source),
        })?;
        match (part.kind.as_deref(), part.image_url) {
            (Some("text"), _) => texts.push(part.text.unwrap_or_default()),
            (
                Some("image_url"),
                Some(ImageUrl::Link(url) | ImageUrl::Fields { url: Some(url) }),
            ) => {
                if !url.is_empty()
                    && let Some(file_id) = uploader.upload(&url).await
                {
                    attachments.push(file_id);
                }
            }
            _ => {}
        }
    }
    Ok((texts.join("\n"), attachments))
}
