use serde::Deserialize;

use crate::panw_prisma_airs::tool_scan::ToolCall;

// The GenericGuardrailAPIInputs slice this guardrail reads and rewrites; images and tools pass through untouched.
#[derive(Debug, Deserialize)]
pub struct Inputs {
    #[serde(default)]
    pub texts: Vec<String>,
    pub structured_messages: Option<Vec<StructuredMessage>>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StructuredMessage {
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
    pub text: Option<String>,
}

#[must_use]
pub fn message_texts(message: &StructuredMessage) -> Vec<&str> {
    match message.content.as_ref() {
        Some(MessageContent::Text(text)) => vec![text.as_str()],
        Some(MessageContent::Parts(parts)) => parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect(),
        None => Vec::new(),
    }
}
