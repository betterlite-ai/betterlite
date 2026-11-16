use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};
use serde_with::{As, DefaultOnNull};

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "MessageRecordFields")]
pub struct MessageRecord {
    pub role: MessageRole,
    pub content: Option<Content>,
}

#[derive(Debug, Clone, Deserialize)]
struct MessageRecordFields {
    role: MessageRole,
    #[serde(default)]
    content: Option<Content>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a content block of a user or assistant message has no `type`")]
pub struct MissingBlockTypeError;

impl TryFrom<MessageRecordFields> for MessageRecord {
    type Error = MissingBlockTypeError;

    fn try_from(fields: MessageRecordFields) -> Result<Self, Self::Error> {
        let untyped = matches!(
            &fields.content,
            Some(Content::Blocks(blocks))
                if blocks.iter().any(|block| matches!(block, Block::Untyped(_)))
        );
        // AnthropicSystemMessageContent is the one content type whose blocks may omit `type`
        if untyped && fields.role != MessageRole::System {
            return Err(MissingBlockTypeError);
        }
        Ok(Self {
            role: fields.role,
            content: fields.content,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Block {
    Known(KnownBlock),
    // AnthropicSystemMessageContent declares `type` optional, so a mid-turn system block may omit it
    Untyped(UntypedBlock),
}

// An unknown block type is not one of the blocks this translation reads, so it leaves the message untouched.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnownBlock {
    Text(TextBlock),
    Image(ImageBlock),
    Document(DocumentBlock),
    ToolResult(ToolResultBlock),
    ToolUse(ToolUseBlock),
    Thinking(ThinkingBlockRecord),
    RedactedThinking(RedactedThinkingBlock),
    #[serde(other)]
    Other,
}

// `type` stays unknown here, so a malformed typed block fails instead of falling through to this arm.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UntypedBlock {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub text: String,
    #[serde(default)]
    pub cache_control: Option<Object>,
    #[serde(default)]
    pub prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextBlock {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub text: String,
    #[serde(default)]
    pub prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageBlock {
    #[serde(default)]
    pub source: Option<ImageSource>,
    #[serde(default)]
    pub prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
}

// LiteLLM's image sources are base64 and url; a source it does not name is no URL.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 {
        #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
        media_type: ImageMediaType,
        #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
        data: String,
    },
    Url {
        #[serde(default)]
        url: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct ImageMediaType(pub String);

impl Default for ImageMediaType {
    fn default() -> Self {
        Self("image/jpeg".to_owned())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DocumentBlock {
    #[serde(default)]
    pub source: Option<DocumentSource>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub prompt_cache_breakpoint: Option<PromptCacheBreakpoint>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DocumentSource {
    Base64 {
        #[serde(default)]
        media_type: Option<String>,
        #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
        data: String,
    },
    Url {
        #[serde(default)]
        url: Option<String>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolResultBlock {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub tool_use_id: String,
    #[serde(default)]
    pub content: Option<ToolResultContent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ToolResultContent {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolUseBlock {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub id: String,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub name: String,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub input: Object,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThinkingBlockRecord {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub thinking: String,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedactedThinkingBlock {
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct PromptCacheBreakpoint {
    pub mode: PromptCacheMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheMode {
    Explicit,
}
