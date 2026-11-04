mod guardrails;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Message {
    pub role: Option<String>,
    pub content: Option<MessageContent>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Response {
    pub choices: Option<Vec<ResponseChoice>>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseChoice {
    pub message: Option<ResponseMessage>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    pub content: Option<MessageContent>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub function_call: Option<FunctionCall>,
}

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub function: Option<FunctionCall>,
}

#[derive(Debug, Deserialize)]
pub struct FunctionCall {
    pub arguments: Option<String>,
}

#[must_use]
pub fn extract_text_from_messages(messages: &[Message]) -> String {
    for message in messages.iter().rev() {
        if !matches!(message.role.as_deref(), Some("user" | "developer")) {
            continue;
        }
        match message.content.as_ref() {
            Some(MessageContent::Text(text)) if !text.is_empty() => return text.clone(),
            Some(MessageContent::Parts(parts)) if !parts.is_empty() => {
                return flatten_text_parts(parts);
            }
            _ => {}
        }
    }
    String::new()
}

fn flatten_text_parts(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter(|part| part.kind.as_deref() == Some("text"))
        .filter_map(|part| part.text.as_deref())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn content_text(content: &MessageContent) -> Option<String> {
    let text = match content {
        MessageContent::Text(text) => text.clone(),
        MessageContent::Parts(parts) => flatten_text_parts(parts),
    };
    if text.is_empty() {
        return None;
    }
    Some(text)
}

pub fn extract_response_text(response: &Response) -> String {
    let mut text = Vec::new();
    for message in response
        .choices
        .iter()
        .flatten()
        .filter_map(|choice| choice.message.as_ref())
    {
        if let Some(content) = message.content.as_ref().and_then(content_text) {
            text.push(content);
        }
        for function in message
            .tool_calls
            .iter()
            .flatten()
            .filter_map(|call| call.function.as_ref())
            .chain(message.function_call.iter())
        {
            if let Some(arguments) = function.arguments.as_ref() {
                text.push(arguments.clone());
            }
        }
    }
    text.join(" ")
}
