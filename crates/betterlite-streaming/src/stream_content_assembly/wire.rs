use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Chunk {
    pub choices: Vec<Choice>,
}
#[derive(Debug, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub delta: Option<Delta>,
}

#[derive(Debug, Deserialize, Default)]
pub struct Delta {
    #[serde(default)]
    pub content: Option<ChunkText>,
    #[serde(default)]
    pub reasoning_content: Option<ChunkText>,
    #[serde(default)]
    pub thinking_blocks: Option<ThinkingBlocks>,
    #[serde(default)]
    pub audio: Option<AudioDelta>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ChunkText {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ThinkingBlocks {
    List(Vec<ThinkingInput>),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
pub struct ThinkingInput {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AudioDelta {
    #[serde(default)]
    pub data: Option<ChunkText>,
    #[serde(default)]
    pub transcript: Option<ChunkText>,
    #[serde(default)]
    pub expires_at: Option<ExpiryInput>,
    #[serde(default)]
    pub id: Option<ChunkText>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone)]
#[serde(untagged)]
pub enum ExpiryValue {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum ExpiryInput {
    Integer(ExpiryValue),
    Other(IgnoredAny),
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ThinkingBlock {
    #[serde(rename = "thinking")]
    Thinking { thinking: String, signature: String },
    #[serde(rename = "redacted_thinking")]
    Redacted { data: String },
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AudioResponse {
    pub data: String,
    pub expires_at: ExpiryValue,
    pub transcript: String,
    pub id: Option<String>,
}
