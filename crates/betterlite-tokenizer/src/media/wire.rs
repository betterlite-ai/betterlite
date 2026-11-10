use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

use crate::image::tokens::Mode;

use super::error::MediaError;

pub type Object = IndexMap<String, Json>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenMedia {
    pub content: Vec<Box<RawValue>>,
}

impl TryFrom<&[u8]> for TokenMedia {
    type Error = MediaError;

    fn try_from(bytes: &[u8]) -> Result<Self, MediaError> {
        serde_json::from_slice(bytes).map_err(|source| MediaError::Json { source })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum ContentItem {
    Text(String),
    Block(Block),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Block {
    Text {
        #[serde(default)]
        text: String,
    },
    ImageUrl {
        image_url: ImageUrlValue,
    },
    Image {
        source: Source,
    },
    Document {
        source: Source,
        #[serde(default)]
        title: String,
        #[serde(default)]
        context: String,
    },
    File {
        file: FileObject,
    },
    ToolUse {
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: Option<Object>,
        #[serde(default)]
        caller: Option<Object>,
    },
    ToolResult {
        #[serde(rename = "tool_use_id")]
        _tool_use_id: String,
        #[serde(default)]
        content: Option<NestedContent>,
    },
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    RedactedThinking,
    ToolReference {
        #[serde(default)]
        tool_name: String,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct BlockKind {
    #[serde(rename = "type")]
    pub(crate) kind: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum ImageUrlValue {
    Text(String),
    Object(ImageUrlObject),
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ImageUrlObject {
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) detail: Mode,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FileObject {
    #[serde(default)]
    pub(crate) filename: String,
    #[serde(default)]
    pub(crate) file_data: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub(crate) source: Source,
    pub(crate) title: String,
    pub(crate) context: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Source {
    Base64,
    File,
    Url {
        #[serde(default)]
        url: String,
    },
    Text {
        data: String,
    },
    Content {
        content: NestedContent,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum NestedContent {
    Text(String),
    Items(Vec<ContentItem>),
    // LiteLLM counts a list inside try/except, so an unreadable one takes the fallback alone
    Unreadable(IgnoredAny),
}
