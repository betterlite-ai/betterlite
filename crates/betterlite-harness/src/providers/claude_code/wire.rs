use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::types::JsonObject;

pub(super) const SYNTHETIC_MODEL: &str = "<synthetic>";

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum Line {
    #[serde(rename = "assistant")]
    Assistant(AssistantLine),
    #[serde(rename = "user")]
    User(UserLine),
    #[serde(rename = "system")]
    System(SystemLine),
    #[serde(rename = "result")]
    Result(ResultLine),
    #[serde(other)]
    Other,
}

pub(super) fn parse(line: &JsonObject) -> Option<Line> {
    let text = serde_json::to_string(line).ok()?;
    serde_json::from_str(&text).ok()
}

#[derive(Deserialize)]
pub(super) struct AssistantLine {
    #[serde(default)]
    pub(super) parent_tool_use_id: Option<String>,
    pub(super) message: AssistantMessage,
}

#[derive(Deserialize)]
pub(super) struct AssistantMessage {
    #[serde(default)]
    pub(super) model: Option<String>,
    pub(super) content: MessageContent,
}

#[derive(Deserialize)]
pub(super) struct UserLine {
    #[serde(default)]
    pub(super) parent_tool_use_id: Option<String>,
    pub(super) message: UserMessage,
}

#[derive(Deserialize)]
pub(super) struct UserMessage {
    pub(super) content: MessageContent,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum MessageContent {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(super) enum Block {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        text: String,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: JsonObject,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: ToolOutput,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
pub(super) enum ToolOutput {
    #[default]
    Empty,
    Text(String),
    Blocks(Vec<ToolOutputBlock>),
}

impl ToolOutput {
    pub(super) fn stringify(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(text) => text.clone(),
            Self::Blocks(blocks) => blocks
                .iter()
                .map(ToolOutputBlock::stringify)
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum ToolOutputBlock {
    Text(String),
    Object(ToolOutputBlockObject),
}

impl ToolOutputBlock {
    fn stringify(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Object(object) => object.text_content(),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct ToolOutputBlockObject {
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) text: Option<String>,
}

impl ToolOutputBlockObject {
    fn text_content(&self) -> String {
        if self.kind.as_deref() == Some("text") {
            return self.text.clone().unwrap_or_default();
        }
        String::new()
    }
}

#[derive(Deserialize)]
pub(super) struct SystemLine {
    #[serde(default)]
    pub(super) subtype: Option<String>,
    #[serde(default)]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) compact_metadata: Option<CompactMetadata>,
}

#[derive(Deserialize)]
pub(super) struct CompactMetadata {
    #[serde(default)]
    pub(super) pre_tokens: Option<u64>,
}

#[derive(Deserialize)]
pub(super) struct ResultLine {
    #[serde(default)]
    pub(super) is_error: bool,
    #[serde(default)]
    pub(super) result: Option<String>,
    #[serde(default)]
    pub(super) errors: Option<Vec<String>>,
    #[serde(default)]
    pub(super) structured_output: Option<StructuredOutputField>,
    #[serde(default)]
    pub(super) session_id: Option<String>,
}

// state.structured_output is only set when the CLI's structured_output is a JSON object; see pitfalls
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum StructuredOutputField {
    Dict(JsonObject),
    Other(IgnoredAny),
}

impl StructuredOutputField {
    pub(super) fn into_dict(self) -> Option<JsonObject> {
        match self {
            Self::Dict(object) => Some(object),
            Self::Other(_) => None,
        }
    }
}
