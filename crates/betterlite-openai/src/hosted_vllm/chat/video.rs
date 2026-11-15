use betterlite_prompts::bedrock_images::parse::parse_mime_type;
use betterlite_prompts::media_metadata::url_mime::image_mime_type_from_url;
use indexmap::IndexMap;
use serde_json::Value;

use super::error::HostedVllmChatError;
use crate::chat::ContentPart;
use crate::openai_base_config::Object;

const FILE: &str = "file";
const FORMAT: &str = "format";
const FILE_DATA: &str = "file_data";
const FILE_ID: &str = "file_id";
const VIDEO_PREFIX: &str = "video/";
const VIDEO_URL: &str = "video_url";

struct FileFields<'part> {
    format: Option<&'part str>,
    file_data: Option<&'part str>,
    file_id: Option<&'part str>,
}

fn text_field<'file>(
    file: &'file Object,
    name: &'static str,
) -> Result<Option<&'file str>, HostedVllmChatError> {
    match file.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(HostedVllmChatError::PartString {
            part: "file",
            field: name,
        }),
    }
}

fn file_fields(part: &ContentPart) -> Result<FileFields<'_>, HostedVllmChatError> {
    let file = match part.fields.get(FILE) {
        None | Some(Value::Null) => None,
        Some(Value::Object(file)) => Some(file),
        Some(_) => return Err(HostedVllmChatError::FileObject),
    };
    let Some(file_object) = file else {
        return Ok(FileFields {
            format: None,
            file_data: None,
            file_id: None,
        });
    };
    Ok(FileFields {
        format: text_field(file_object, FORMAT)?,
        file_data: text_field(file_object, FILE_DATA)?,
        file_id: text_field(file_object, FILE_ID)?,
    })
}

// a video format answers before any source is read, and a present file_data is the only source checked next
pub(crate) fn is_video_file(part: &ContentPart) -> Result<bool, HostedVllmChatError> {
    if part.kind != FILE {
        return Ok(false);
    }
    let fields = file_fields(part)?;
    if let Some(format) = fields.format.filter(|format| !format.is_empty())
        && format.starts_with(VIDEO_PREFIX)
    {
        return Ok(true);
    }
    if let Some(file_data) = fields.file_data.filter(|data| !data.is_empty()) {
        return Ok(parse_mime_type(file_data).is_some_and(|mime| mime.starts_with(VIDEO_PREFIX)));
    }
    if let Some(file_id) = fields.file_id.filter(|id| !id.is_empty()) {
        return Ok(
            image_mime_type_from_url(file_id).is_some_and(|mime| mime.starts_with(VIDEO_PREFIX))
        );
    }
    Ok(false)
}

pub(crate) fn convert_file_to_video_url(
    part: &ContentPart,
) -> Result<ContentPart, HostedVllmChatError> {
    let fields = file_fields(part)?;
    let url = match (
        fields.file_id.filter(|id| !id.is_empty()),
        fields.file_data.filter(|data| !data.is_empty()),
    ) {
        (Some(file_id), _) => file_id,
        (None, Some(file_data)) => file_data,
        (None, None) => return Err(HostedVllmChatError::MissingVideoSource),
    };
    let video_url = Value::Object(Object::from_iter([(
        String::from("url"),
        Value::String(url.to_owned()),
    )]));
    Ok(ContentPart {
        kind: String::from(VIDEO_URL),
        fields: IndexMap::from([(String::from(VIDEO_URL), video_url)]),
    })
}
