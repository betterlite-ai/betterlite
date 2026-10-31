use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::rust::double_option;

use super::content::{Content, ReasoningText, ThinkingBlocks};
use super::{Reasoning, Role, is_falsy};

// A chat message, dict or attribute shaped, with double_option keeping "missing" distinct from an explicit null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRecord {
    #[serde(default, skip_serializing_if = "Role::is_missing")]
    pub role: Role,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub content: Option<Option<Content>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub reasoning_content: Option<Option<ReasoningText>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub thinking_blocks: Option<Option<ThinkingBlocks>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "double_option"
    )]
    pub tool_calls: Option<Option<Value>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Reasoning for ChatRecord {
    fn is_pending(&self) -> bool {
        matches!(self.role, Role::Known(super::KnownRole::Assistant))
            && self.content.as_ref().and_then(Option::as_ref).is_none()
            && self
                .tool_calls
                .as_ref()
                .and_then(Option::as_ref)
                .is_none_or(is_falsy)
            && (self.text().is_some() || self.blocks().is_some())
    }

    fn is_assistant(&self) -> bool {
        matches!(self.role, Role::Known(super::KnownRole::Assistant))
    }

    fn text(&self) -> Option<&str> {
        self.reasoning_content
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(ReasoningText::as_text)
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.thinking_blocks
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(ThinkingBlocks::as_list)
    }

    fn set_text(&mut self, text: String) {
        self.reasoning_content = Some(Some(ReasoningText::Text(text)));
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.thinking_blocks = Some(Some(ThinkingBlocks::List(blocks)));
    }

    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        let mut record = Self {
            role: Role::ASSISTANT,
            content: Some(None),
            reasoning_content: None,
            thinking_blocks: None,
            tool_calls: None,
            extra: Map::new(),
        };
        if let Some(found) = text.filter(|candidate| !candidate.is_empty()) {
            record.set_text(found);
        }
        if let Some(found) = blocks.filter(|candidate| !candidate.is_empty()) {
            record.set_blocks(found);
        }
        record
    }
}

// Tags dict and attribute objects because LiteLLM dispatches each message independently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Dict(ChatRecord),
    Attribute(ChatRecord),
}

impl Record {
    const fn inner(&self) -> &ChatRecord {
        let (Self::Dict(record) | Self::Attribute(record)) = self;
        record
    }

    const fn inner_mut(&mut self) -> &mut ChatRecord {
        let (Self::Dict(record) | Self::Attribute(record)) = self;
        record
    }
}

impl Reasoning for Record {
    fn is_pending(&self) -> bool {
        self.inner().is_pending()
    }

    fn is_assistant(&self) -> bool {
        self.inner().is_assistant()
    }

    fn text(&self) -> Option<&str> {
        self.inner().text()
    }

    fn blocks(&self) -> Option<&[Value]> {
        self.inner().blocks()
    }

    fn set_text(&mut self, text: String) {
        self.inner_mut().set_text(text);
    }

    fn set_blocks(&mut self, blocks: Vec<Value>) {
        self.inner_mut().set_blocks(blocks);
    }

    fn blank(text: Option<String>, blocks: Option<Vec<Value>>) -> Self {
        Self::Dict(ChatRecord::blank(text, blocks))
    }
}
