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

impl ContentItem {
    pub(crate) fn from_json(value: &Json) -> Result<Self, MediaError> {
        if let Json::Text(text) = value {
            return Ok(Self::Text(text.clone()));
        }
        match value {
            Json::Object(object) => Ok(Self::Block(RawBlock::from_object(object)?)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Text(_) => {
                Err(MediaError::ContentItem)
            }
        }
    }
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
        source: Source,
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
        content: Option<NestedContent>,
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
    pub(crate) source: Source,
    pub(crate) title: Option<String>,
    pub(crate) context: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum Source {
    Base64,
    File,
    Url { url: Option<String> },
    Text { data: String },
    Content(NestedContent),
}

#[derive(Debug, Clone)]
pub(crate) enum NestedContent {
    Text(String),
    Items(Vec<ContentItem>),
}
