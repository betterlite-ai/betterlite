use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::image::tokens::Mode;

use super::error::MediaError;

pub type Object = IndexMap<String, Json>;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenMedia {
    pub content: Vec<ContentItem>,
}

impl TryFrom<&[u8]> for TokenMedia {
    type Error = MediaError;

    fn try_from(bytes: &[u8]) -> Result<Self, MediaError> {
        serde_json::from_slice(bytes).map_err(|source| MediaError::Json { source })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ContentItem {
    Text(String),
    Block(RawBlock),
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawBlock {
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
    #[serde(flatten)]
    pub(crate) fields: Object,
}

#[derive(Debug, Clone)]
pub(crate) enum Block<'block> {
    Text {
        text: String,
    },
    ImageUrl {
        image: ImageUrl,
    },
    Image {
        source: ImageSource,
    },
    Document(Document),
    File {
        filename: Option<String>,
        file_data: Option<String>,
    },
    ToolUse {
        name: Option<String>,
        input: Option<&'block Object>,
        caller: Option<&'block Object>,
    },
    ToolResult {
        content: Option<ToolContent>,
    },
    Thinking {
        text: Option<String>,
    },
    ToolReference {
        tool_name: Option<String>,
    },
    Unsupported {
        kind: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ImageUrl {
    pub(crate) url: String,
    pub(crate) detail: Mode,
}

#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub(crate) source: DocumentSource,
    pub(crate) title: Option<String>,
    pub(crate) context: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum ImageSource {
    Url { url: Option<String> },
    Local,
}

#[derive(Debug, Clone)]
pub(crate) enum DocumentSource {
    Text { data: String },
    Content(DocumentContent),
    Url { url: Option<String> },
    Local,
}

#[derive(Debug, Clone)]
pub(crate) enum DocumentContent {
    Text(String),
    Items(Vec<ContentItem>),
}

#[derive(Debug, Clone)]
pub(crate) enum ToolContent {
    Text(String),
    Items(Vec<ContentItem>),
}
