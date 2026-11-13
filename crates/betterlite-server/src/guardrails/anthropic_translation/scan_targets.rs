#![expect(
    clippy::disallowed_types,
    reason = "Anthropic request blocks and tool_use inputs are provider JSON that betterlite does not model"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json test fixtures built with json! are concise and type-directed"
    )
)]

mod tests;

use serde_json::{Map, Value};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MessageTextTarget {
    MessageContent {
        msg_idx: usize,
    },
    ContentBlockText {
        msg_idx: usize,
        content_idx: usize,
    },
    ToolResultString {
        msg_idx: usize,
        content_idx: usize,
    },
    ToolResultBlockText {
        msg_idx: usize,
        content_idx: usize,
        block_idx: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputWriteBackTarget {
    SystemString,
    SystemBlockText { block_idx: usize },
    Message(MessageTextTarget),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolUseInputTarget {
    pub msg_idx: usize,
    pub content_idx: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedText {
    pub text: String,
    pub target: InputWriteBackTarget,
}

#[derive(Clone, Debug)]
pub struct ScannedToolCall {
    pub tool_call: Value,
    pub target: ToolUseInputTarget,
}

#[derive(Clone, Debug)]
pub struct ExtractedInput {
    pub scanned: Vec<ScannedText>,
    pub images: Vec<String>,
    pub tool_calls: Vec<ScannedToolCall>,
}

impl PartialEq for ScannedToolCall {
    fn eq(&self, other: &Self) -> bool {
        self.tool_call == other.tool_call && self.target == other.target
    }
}

impl PartialEq for ExtractedInput {
    fn eq(&self, other: &Self) -> bool {
        self.scanned == other.scanned
            && self.images == other.images
            && self.tool_calls == other.tool_calls
    }
}

impl ExtractedInput {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            scanned: Vec::new(),
            images: Vec::new(),
            tool_calls: Vec::new(),
        }
    }
}

#[must_use]
pub fn is_client_tool_use(block: &Map<String, Value>) -> bool {
    block.get("type").and_then(Value::as_str) == Some("tool_use")
        && block.get("id").is_some_and(Value::is_string)
        && block.get("name").is_some_and(Value::is_string)
        && block.get("input").is_some_and(Value::is_object)
}
