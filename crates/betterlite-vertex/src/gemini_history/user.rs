use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::limits::MAX_MESSAGE_PARTS;
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use crate::gemini_history::content::{MediaInput, Part, TextPart, push_media};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::fields::{
    DETAIL, mime_hints, optional_string, required_object, required_string,
};

use crate::gemini_media::error::MediaField;
use crate::gemini_media::mime::MimeHints;
use crate::gemini_media::part::VideoMetadata;
use crate::gemini_media::resolution::convert_detail;

const TEXT: &str = "text";
const IMAGE_URL: &str = "image_url";
const INPUT_AUDIO: &str = "input_audio";
const FILE: &str = "file";
const URL: &str = "url";
const DATA: &str = "data";
const FORMAT: &str = "format";
const FILE_ID: &str = "file_id";
const FILE_DATA: &str = "file_data";
const VIDEO_METADATA: &str = "video_metadata";
const AUDIO_TYPE_PREFIX: &str = "audio/";
const AUDIO_URL_PREFIX: &str = "data:";
const BASE64_MARKER: &str = ";base64,";

pub fn append_user_parts(
    message: &Message,
    index: usize,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    match &message.content {
        None => Ok(()),
        Some(Content::Text(text)) => {
            parts.push(Part::Text(TextPart {
                text: text.clone(),
                signature: None,
            }));
            Ok(())
        }
        Some(Content::Parts(items)) => {
            if items.len() > MAX_MESSAGE_PARTS {
                return Err(HistoryError::TooManyParts {
                    index,
                    limit: MAX_MESSAGE_PARTS,
                });
            }
            for (part_index, item) in items.iter().enumerate() {
                append_user_part(item, index, part_index, model, parts)?;
            }
            Ok(())
        }
    }
}

fn append_user_part(
    item: &ContentPart,
    message_index: usize,
    part_index: usize,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let path = format!("messages[{message_index}].content[{part_index}]");
    match item.kind.as_str() {
        TEXT => {
            let Some(value) = item.fields.get(TEXT) else {
                return Ok(());
            };
            let Json::Text(text) = value else {
                return Err(HistoryError::WrongField {
                    path: format!("{path}.text"),
                    expected: "a string",
                });
            };
            if !text.is_empty() {
                parts.push(Part::Text(TextPart {
                    text: text.clone(),
                    signature: None,
                }));
            }
            Ok(())
        }
        IMAGE_URL => append_image(item, &path, model, parts),
        INPUT_AUDIO => append_audio(item, &path, model, parts),
        FILE => append_file(item, &path, model, parts),
        _ => Ok(()),
    }
}

fn append_image(
    item: &ContentPart,
    path: &str,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let Some(raw) = item.fields.get(IMAGE_URL) else {
        return Err(HistoryError::MissingField {
            path: format!("{path}.image_url"),
        });
    };
    match raw {
        Json::Null => Err(HistoryError::MissingField {
            path: format!("{path}.image_url"),
        }),
        Json::Text(url) => {
            let hints = MimeHints::default();
            let media = MediaInput {
                url,
                hints: &hints,
                resolution: None,
                video_metadata: None,
                model,
                field: MediaField::ImageUrl,
            };
            push_media(parts, &media)
        }
        Json::Object(fields) => image_fields(fields, path, model, parts),
        Json::Bool(_) | Json::Number(_) | Json::List(_) => Err(HistoryError::WrongField {
            path: format!("{path}.image_url"),
            expected: "a string or an object",
        }),
    }
}

fn image_fields(
    fields: &Object,
    path: &str,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let url = required_string(fields, URL, &format!("{path}.image_url"))?;
    let hints = mime_hints(fields, &format!("{path}.image_url"))?;
    let resolution = optional_string(fields, DETAIL, &format!("{path}.image_url"))?
        .as_deref()
        .and_then(convert_detail);
    let media = MediaInput {
        url: &url,
        hints: &hints,
        resolution,
        video_metadata: None,
        model,
        field: MediaField::ImageUrl,
    };
    push_media(parts, &media)
}

fn append_audio(
    item: &ContentPart,
    path: &str,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let fields = required_object(&item.fields, INPUT_AUDIO, path)?;
    let audio_path = format!("{path}.input_audio");
    let Some(data) = optional_string(fields, DATA, &audio_path)? else {
        return Ok(());
    };
    let Some(format) = optional_string(fields, FORMAT, &audio_path)? else {
        return Ok(());
    };
    let media_type = if format.starts_with(AUDIO_TYPE_PREFIX) {
        format
    } else {
        format!("{AUDIO_TYPE_PREFIX}{format}")
    };
    let url = format!("{AUDIO_URL_PREFIX}{media_type}{BASE64_MARKER}{data}");
    let hints = MimeHints {
        format: Some(media_type),
        mime_type: None,
        content_type: None,
    };
    let media = MediaInput {
        url: &url,
        hints: &hints,
        resolution: None,
        video_metadata: None,
        model,
        field: MediaField::ImageUrl,
    };
    push_media(parts, &media)
}

fn append_file(
    item: &ContentPart,
    path: &str,
    model: Option<&str>,
    parts: &mut Vec<Part>,
) -> Result<(), HistoryError> {
    let fields = required_object(&item.fields, FILE, path)?;
    let file_path = format!("{path}.file");
    let file_id = optional_string(fields, FILE_ID, &file_path)?;
    let file_data = optional_string(fields, FILE_DATA, &file_path)?;
    let Some(url) = file_id.or(file_data) else {
        return Err(HistoryError::WrongField {
            path: file_path,
            expected: "a file_id or file_data",
        });
    };
    let hints = mime_hints(fields, &file_path)?;
    let resolution = optional_string(fields, DETAIL, &file_path)?
        .as_deref()
        .and_then(convert_detail);
    let video_metadata = match fields.get(VIDEO_METADATA) {
        None | Some(Json::Null) => None,
        Some(value) => {
            let encoded =
                serde_json::to_value(value).map_err(|source| HistoryError::Encode { source })?;
            let decoded = serde_json::from_value::<VideoMetadata>(encoded).map_err(|source| {
                HistoryError::VideoMetadata {
                    path: format!("{file_path}.video_metadata"),
                    source,
                }
            })?;
            Some(decoded)
        }
    };
    let media = MediaInput {
        url: &url,
        hints: &hints,
        resolution,
        video_metadata: video_metadata.as_ref(),
        model,
        field: MediaField::File,
    };
    push_media(parts, &media)
}
