use betterlite_models::decisions::Json;

use crate::image::tokens::Mode;

use super::error::MediaError;
use super::fields::{FieldResult, object, require, required_text, text};
use super::wire::{
    Block, ContentItem, Document, ImageUrl, NestedContent, Object, RawBlock, Source,
};

impl RawBlock {
    pub(crate) fn from_object(object: &Object) -> FieldResult<Self> {
        let kind = match object.get("type") {
            Some(Json::Text(kind)) => Some(kind.clone()),
            None | Some(Json::Null) => None,
            Some(_) => return Err(MediaError::Field { field: "type" }),
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
                text: text(fields, "text")?.unwrap_or_default(),
            }),
            "image_url" => Ok(Block::ImageUrl {
                image: image_url(fields)?,
            }),
            "image" => Ok(Block::Image {
                source: source_of(fields)?,
            }),
            "thinking" | "redacted_thinking" => Ok(Block::Thinking {
                text: text(fields, "thinking")?,
            }),
            "tool_reference" => Ok(Block::ToolReference {
                tool_name: text(fields, "tool_name")?,
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

fn document_block(fields: &Object) -> FieldResult<Block<'_>> {
    Ok(Block::Document(Document {
        source: source_of(fields)?,
        title: text(fields, "title")?,
        context: text(fields, "context")?,
    }))
}

fn file_block(fields: &Object) -> FieldResult<Block<'_>> {
    let Some(file) = object(fields, "file")? else {
        return Err(MediaError::Field { field: "file" });
    };
    Ok(Block::File {
        filename: text(file, "filename")?,
        file_data: text(file, "file_data")?,
    })
}

fn tool_use_block(fields: &Object) -> FieldResult<Block<'_>> {
    Ok(Block::ToolUse {
        name: text(fields, "name")?,
        input: object(fields, "input")?,
        caller: object(fields, "caller")?,
    })
}

fn tool_result_block(fields: &Object) -> FieldResult<Block<'_>> {
    require(fields, "tool_use_id")?;
    Ok(Block::ToolResult {
        content: optional_nested(fields, "content")?,
    })
}

fn nested(value: &Json, name: &'static str) -> FieldResult<NestedContent> {
    match value {
        Json::Text(text) => Ok(NestedContent::Text(text.clone())),
        Json::List(items) => Ok(NestedContent::Items(
            items
                .iter()
                .map(ContentItem::from_json)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Err(MediaError::Field { field: name })
        }
    }
}

fn optional_nested(fields: &Object, name: &'static str) -> FieldResult<Option<NestedContent>> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(nested(value, name)?)),
    }
}

fn source_of(fields: &Object) -> FieldResult<Source> {
    let Some(source) = object(fields, "source")? else {
        return Err(MediaError::Field { field: "source" });
    };
    let Some(kind) = text(source, "type")? else {
        return Err(MediaError::Field { field: "type" });
    };
    match kind.as_str() {
        "base64" => Ok(Source::Base64),
        "file" => Ok(Source::File),
        "url" => Ok(Source::Url {
            url: text(source, "url")?,
        }),
        "text" => Ok(Source::Text {
            data: required_text(source, "data")?,
        }),
        "content" => nested_source(source),
        other => Err(MediaError::UnsupportedSource {
            kind: other.to_owned(),
        }),
    }
}

fn nested_source(source: &Object) -> FieldResult<Source> {
    let Some(value) = source.get("content") else {
        return Err(MediaError::Field { field: "content" });
    };
    Ok(Source::Content(nested(value, "content")?))
}

fn image_url(fields: &Object) -> FieldResult<ImageUrl> {
    match fields.get("image_url") {
        Some(Json::Text(text)) if !text.trim().is_empty() => Ok(ImageUrl {
            url: text.clone(),
            detail: Mode::Auto,
        }),
        Some(Json::Object(object)) => image_url_object(object),
        None
        | Some(Json::Null | Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            Err(MediaError::Field { field: "image_url" })
        }
    }
}

fn image_url_object(object: &Object) -> FieldResult<ImageUrl> {
    let Some(url) = text(object, "url")?.filter(|url| !url.is_empty()) else {
        return Err(MediaError::Field { field: "url" });
    };
    let detail = detail(object)?;
    Ok(ImageUrl { url, detail })
}

fn detail(object: &Object) -> FieldResult<Mode> {
    match text(object, "detail")?.as_deref() {
        None | Some("auto") => Ok(Mode::Auto),
        Some("low") => Ok(Mode::Low),
        Some("high") => Ok(Mode::High),
        Some(_) => Err(MediaError::Field { field: "detail" }),
    }
}
