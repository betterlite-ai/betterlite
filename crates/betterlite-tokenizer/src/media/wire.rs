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
    fn from_json(value: &Json) -> Result<Self, MediaError> {
        match value {
            Json::Text(text) => Ok(Self::Text(text.clone())),
            Json::Object(object) => Ok(Self::Block(RawBlock::from_object(object)?)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                Err(MediaError::ContentItem)
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawBlock {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(flatten)]
    fields: Object,
}

impl RawBlock {
    fn from_object(object: &Object) -> Result<Self, MediaError> {
        let kind = match object.get("type") {
            Some(Json::Text(kind)) => Some(kind.clone()),
            None | Some(Json::Null) => None,
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(MediaError::Field {
                    block: "content block",
                    field: "type",
                });
            }
        };
        Ok(Self {
            kind,
            fields: object.clone(),
        })
    }

    pub(crate) fn parse(&self) -> Result<Block<'_>, MediaError> {
        let Some(kind) = self.kind.as_deref() else {
            return Err(MediaError::MissingType);
        };
        let fields = &self.fields;
        match kind {
            "text" => Ok(Block::Text {
                text: optional_text(fields, "text", "text")?.unwrap_or_default(),
            }),
            "image_url" => Ok(Block::ImageUrl {
                image: image_url(fields)?,
            }),
            "image" => Ok(Block::Image {
                source: image_source(required_object(fields, "image", "source")?)?,
            }),
            "thinking" | "redacted_thinking" => Ok(Block::Thinking {
                text: optional_text(fields, "thinking", "thinking")?,
            }),
            "tool_reference" => Ok(Block::ToolReference {
                tool_name: optional_text(fields, "tool_reference", "tool_name")?,
            }),
            "document" => document_block(fields),
            "file" => file_block(fields),
            "tool_use" => tool_use_block(fields),
            "tool_result" => tool_result_block(fields),
            other => Ok(Block::Unsupported {
                kind: other.to_owned(),
            }),
        }
    }
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

fn document_block(fields: &Object) -> Result<Block<'_>, MediaError> {
    Ok(Block::Document(Document {
        source: document_source(required_object(fields, "document", "source")?)?,
        title: optional_text(fields, "document", "title")?,
        context: optional_text(fields, "document", "context")?,
    }))
}

fn file_block(fields: &Object) -> Result<Block<'_>, MediaError> {
    let file = required_object(fields, "file", "file")?;
    Ok(Block::File {
        filename: optional_text(file, "file", "filename")?,
        file_data: optional_text(file, "file", "file_data")?,
    })
}

fn tool_use_block(fields: &Object) -> Result<Block<'_>, MediaError> {
    Ok(Block::ToolUse {
        name: optional_text(fields, "tool_use", "name")?,
        input: optional_object(fields, "tool_use", "input")?,
        caller: optional_object(fields, "tool_use", "caller")?,
    })
}

fn tool_result_block(fields: &Object) -> Result<Block<'_>, MediaError> {
    required_text(fields, "tool_result", "tool_use_id")?;
    Ok(Block::ToolResult {
        content: optional_content(fields)?,
    })
}

fn optional_text(
    fields: &Object,
    block: &'static str,
    field: &'static str,
) -> Result<Option<String>, MediaError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(MediaError::Field { block, field })
        }
    }
}

fn required_text(
    fields: &Object,
    block: &'static str,
    field: &'static str,
) -> Result<String, MediaError> {
    match optional_text(fields, block, field)? {
        Some(text) => Ok(text),
        None => Err(MediaError::Field { block, field }),
    }
}

fn required_object<'fields>(
    fields: &'fields Object,
    block: &'static str,
    field: &'static str,
) -> Result<&'fields Object, MediaError> {
    match fields.get(field) {
        Some(Json::Object(object)) => Ok(object),
        None
        | Some(Json::Null | Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(MediaError::Field { block, field })
        }
    }
}

fn optional_object<'fields>(
    fields: &'fields Object,
    block: &'static str,
    field: &'static str,
) -> Result<Option<&'fields Object>, MediaError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(object)) => Ok(Some(object)),
        Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(MediaError::Field { block, field })
        }
    }
}

fn image_url(fields: &Object) -> Result<ImageUrl, MediaError> {
    match fields.get("image_url") {
        Some(Json::Text(text)) if !text.trim().is_empty() => Ok(ImageUrl {
            url: text.clone(),
            detail: Mode::Auto,
        }),
        Some(Json::Object(object)) => {
            let url = required_text(object, "image_url", "url")?;
            if url.is_empty() {
                return Err(MediaError::Field {
                    block: "image_url",
                    field: "url",
                });
            }
            Ok(ImageUrl {
                url,
                detail: image_detail(object)?,
            })
        }
        None
        | Some(Json::Null | Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(MediaError::Field {
                block: "image_url",
                field: "image_url",
            })
        }
    }
}

fn image_detail(object: &Object) -> Result<Mode, MediaError> {
    match object.get("detail") {
        None | Some(Json::Null) => Ok(Mode::Auto),
        Some(Json::Text(text)) => match text.as_str() {
            "low" => Ok(Mode::Low),
            "high" => Ok(Mode::High),
            "auto" => Ok(Mode::Auto),
            _ => Err(MediaError::Field {
                block: "image_url",
                field: "detail",
            }),
        },
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(MediaError::Field {
                block: "image_url",
                field: "detail",
            })
        }
    }
}

fn image_source(source: &Object) -> Result<ImageSource, MediaError> {
    let kind = required_text(source, "image source", "type")?;
    match kind.as_str() {
        "url" => Ok(ImageSource::Url {
            url: optional_text(source, "image source", "url")?,
        }),
        "base64" | "file" => Ok(ImageSource::Local),
        other => Err(MediaError::UnsupportedSource {
            kind: other.to_owned(),
        }),
    }
}

fn document_source(source: &Object) -> Result<DocumentSource, MediaError> {
    let kind = required_text(source, "document source", "type")?;
    match kind.as_str() {
        "text" => Ok(DocumentSource::Text {
            data: required_text(source, "document source", "data")?,
        }),
        "content" => Ok(DocumentSource::Content(document_content(source)?)),
        "url" => Ok(DocumentSource::Url {
            url: optional_text(source, "document source", "url")?,
        }),
        "base64" | "file" => Ok(DocumentSource::Local),
        other => Err(MediaError::UnsupportedSource {
            kind: other.to_owned(),
        }),
    }
}

fn document_content(source: &Object) -> Result<DocumentContent, MediaError> {
    match source.get("content") {
        Some(Json::Text(text)) => Ok(DocumentContent::Text(text.clone())),
        Some(Json::List(items)) => Ok(DocumentContent::Items(
            items
                .iter()
                .map(ContentItem::from_json)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) => {
            Err(MediaError::Field {
                block: "document source",
                field: "content",
            })
        }
    }
}

fn optional_content(fields: &Object) -> Result<Option<ToolContent>, MediaError> {
    match fields.get("content") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(ToolContent::Text(text.clone()))),
        Some(Json::List(items)) => Ok(Some(ToolContent::Items(
            items
                .iter()
                .map(ContentItem::from_json)
                .collect::<Result<Vec<_>, _>>()?,
        ))),
        Some(Json::Bool(_) | Json::Number(_) | Json::Object(_)) => Err(MediaError::Field {
            block: "tool_result",
            field: "content",
        }),
    }
}
