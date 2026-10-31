use super::super::Reasoning;
use serde_json::Value;

// Mirrors how LiteLLM reads a non-dict message through getattr: closed fields, no passthrough.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypedChatRecord {
    pub(super) role: String,
    pub(super) content: Option<String>,
    pub(super) reasoning_content: Option<String>,
    pub(super) thinking_blocks: Option<Vec<Value>>,
    pub(super) tool_calls: Option<Vec<Value>>,
}

impl Reasoning for TypedChatRecord {
    fn is_pending(&self) -> bool {
        self.role == "assistant"
            && self.content.is_none()
            && self.tool_calls.as_ref().is_none_or(Vec::is_empty)
            && (self.text().is_some() || self.blocks().is_some())
    }

    fn is_assistant(&self) -> bool {
        self.role == "assistant"
    }

    fn text(&self) -> Option<&str> {
        self.reasoning_content
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.thinking_blocks
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    fn set_text(&mut self, text: String) {
        self.reasoning_content = Some(text);
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.thinking_blocks = Some(blocks);
    }

    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        Self {
            role: "assistant".to_owned(),
            content: None,
            reasoning_content: text,
            thinking_blocks: blocks,
            tool_calls: None,
        }
    }
}
