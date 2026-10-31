use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("choices must be a list or null")]
pub struct ChoicesError(#[source] serde_json::Error);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    Anthropic,
    OpenAi,
    Responses,
}
impl ResponseFormat {
    #[must_use]
    pub fn parse(format: &str) -> Self {
        match format {
            "openai" => Self::OpenAi,
            "responses" => Self::Responses,
            _ => Self::Anthropic,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, from = "Box<RawValue>")]
pub enum Input {
    Object(IndexMap<String, Box<RawValue>>),
    Other(Box<RawValue>),
}
impl From<Box<RawValue>> for Input {
    fn from(raw: Box<RawValue>) -> Self {
        serde_json::from_str(raw.get()).map_or_else(|_| Self::Other(raw), Self::Object)
    }
}
impl Default for Input {
    fn default() -> Self {
        Self::Object(IndexMap::new())
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub content: Blocks,
    #[serde(default)]
    pub choices: Choices,
    #[serde(default)]
    pub output: Blocks,
}
#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "Box<RawValue>")]
pub struct Choices(pub Vec<Choice>);
impl TryFrom<Box<RawValue>> for Choices {
    type Error = ChoicesError;
    fn try_from(raw: Box<RawValue>) -> Result<Self, Self::Error> {
        if raw.get() == "null" {
            return Ok(Self(Vec::new()));
        }
        let items = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()).map_err(ChoicesError)?;
        let choice = items
            .first()
            .and_then(|first| serde_json::from_str::<Choice>(first.get()).ok());
        Ok(Self(choice.into_iter().collect()))
    }
}
#[derive(Debug, Default, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub struct Blocks(pub Vec<Block>);
impl From<Box<RawValue>> for Blocks {
    fn from(raw: Box<RawValue>) -> Self {
        let items = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()).unwrap_or_default();
        Self(
            items
                .iter()
                .filter_map(|item| serde_json::from_str::<Block>(item.get()).ok())
                .collect(),
        )
    }
}
#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: Option<Message>,
}
#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub tool_calls: Blocks,
}
#[derive(Debug, Deserialize)]
pub struct Block {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub id: Option<String>,
    pub call_id: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub input: Input,
    #[serde(default)]
    pub arguments: Arguments,
    pub function: Option<Function>,
}
#[derive(Debug, Deserialize)]
pub struct Function {
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Arguments,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    ToolUse,
    Function,
    FunctionCall,
}
#[derive(Debug, Clone, Serialize)]
pub struct Call {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: CallKind,
    pub name: String,
    pub input: Input,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CallFunction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<Option<String>>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CallFunction {
    pub name: String,
    pub arguments: Input,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Thinking {
    Thinking { thinking: String, signature: String },
    RedactedThinking { data: String },
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Content {
    Thinking(Thinking),
    ToolUse {
        #[serde(rename = "type")]
        kind: CallKind,
        id: Option<String>,
        name: String,
        input: Input,
    },
}
#[derive(Debug, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Assistant {
    Assistant {
        #[serde(flatten)]
        body: AssistantBody,
    },
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AssistantBody {
    Anthropic { content: Vec<Content> },
    OpenAi { tool_calls: Vec<WireCall> },
}
#[derive(Debug, Serialize)]
pub struct WireCall {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: CallKind,
    pub function: WireFunction,
}
#[derive(Debug, Serialize)]
pub struct WireFunction {
    pub name: String,
    pub arguments: String,
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Results {
    Anthropic(UserMessage),
    OpenAi(Vec<ToolMessage>),
}
#[derive(Debug, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum UserMessage {
    User { content: Vec<ToolResult> },
}
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResult {
    ToolResult {
        tool_use_id: Option<String>,
        content: String,
    },
}
#[derive(Debug, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ToolMessage {
    Tool {
        tool_call_id: Option<String>,
        content: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(from = "Box<RawValue>")]
pub enum Arguments {
    #[default]
    Missing,
    Present(Box<RawValue>),
}
impl From<Box<RawValue>> for Arguments {
    fn from(raw: Box<RawValue>) -> Self {
        Self::Present(raw)
    }
}

impl Assistant {
    pub fn prepend_thinking(&mut self, thinking: &[Thinking]) {
        match self {
            Self::Assistant {
                body: AssistantBody::Anthropic { content },
            } => {
                content.splice(..0, thinking.iter().cloned().map(Content::Thinking));
            }
            Self::Assistant {
                body: AssistantBody::OpenAi { .. },
            } => {}
        }
    }
}
