use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_results::types::LastToolCallMessage;
use betterlite_prompts::message_content::{Content, ContentPart, Message};

use crate::gemini_history::content::{
    CONTENT_TYPE, DETAIL, FORMAT, MIME_TYPE, decode_thinking_part, media_request, object,
    push_media,
};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::tool_turn::{
    append_invocations, append_tool_calls, collect_tool_signatures,
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
const FILE_ID: &str = "file_id";
const FILE_DATA: &str = "file_data";
const VIDEO_METADATA: &str = "video_metadata";
const REASONING_CONTENT: &str = "reasoning_content";
const THINKING_BLOCKS: &str = "thinking_blocks";
const IMAGES: &str = "images";
const PROVIDER_FIELDS: &str = "provider_specific_fields";
const THOUGHT_SIGNATURES: &str = "thought_signatures";

#[derive(Clone, Copy)]
pub struct AssistantOptions<'model> {
    pub model: Option<&'model str>,
    pub forward_function_call_id: bool,
}

fn hints_of(fields: &Object) -> MimeHints {
    MimeHints {
        format: text_of(fields, FORMAT),
        mime_type: text_of(fields, MIME_TYPE),
        content_type: text_of(fields, CONTENT_TYPE),
    }
}

fn text_of(fields: &Object, name: &str) -> Option<String> {
    if let Some(Json::Text(value)) = fields.get(name) {
        return Some(value.clone());
    }
    None
}

pub fn append_user_parts(
    message: &Message,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    match &message.content {
        None => Ok(()),
        Some(Content::Text(text)) => {
            parts.push(object!(TEXT => Json::Text(text.clone())));
            Ok(())
        }
        Some(Content::Parts(items)) => {
            for item in items {
                append_user_part(item, model, parts)?;
            }
            Ok(())
        }
    }
}

fn append_user_part(
    item: &ContentPart,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let fields = &item.fields;
    match item.kind.as_str() {
        TEXT => {
            if let Some(text) = text_of(fields, TEXT)
                && !text.is_empty()
            {
                parts.push(object!(TEXT => Json::Text(text)));
            }
            Ok(())
        }
        IMAGE_URL => append_image(fields, model, parts),
        INPUT_AUDIO => append_audio(fields, model, parts),
        FILE => append_file(fields, model, parts),
        _ => Ok(()),
    }
}

fn append_image(
    fields: &Object,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let (url, hints, resolution) = match fields.get(IMAGE_URL) {
        Some(Json::Text(url)) => (url.clone(), MimeHints::default(), None),
        Some(Json::Object(image)) => {
            let url = text_of(image, URL)
                .ok_or_else(|| HistoryError::Invalid("image_url.url is missing".to_owned()))?;
            let resolution = text_of(image, DETAIL).as_deref().and_then(convert_detail);
            (url, hints_of(image), resolution)
        }
        None | Some(Json::Null) => {
            return Err(HistoryError::Invalid("image_url is missing".to_owned()));
        }
        Some(_) => {
            return Err(HistoryError::Invalid(
                "image_url must be a string or an object".to_owned(),
            ));
        }
    };
    let request = media_request!(&url, &hints, resolution, None, model, MediaField::ImageUrl);
    push_media(parts, &request)
}

fn append_audio(
    fields: &Object,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let Some(Json::Object(audio)) = fields.get(INPUT_AUDIO) else {
        return Err(HistoryError::Invalid("input_audio is missing".to_owned()));
    };
    let (Some(data), Some(format)) = (text_of(audio, DATA), text_of(audio, FORMAT)) else {
        return Ok(());
    };
    let media_type = if format.starts_with("audio/") {
        format
    } else {
        format!("audio/{format}")
    };
    let url = format!("data:{media_type};base64,{data}");
    let hints = MimeHints {
        format: Some(media_type),
        mime_type: None,
        content_type: None,
    };
    let request = media_request!(&url, &hints, None, None, model, MediaField::ImageUrl);
    push_media(parts, &request)
}

fn append_file(
    fields: &Object,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let Some(Json::Object(file)) = fields.get(FILE) else {
        return Err(HistoryError::Invalid("file is missing".to_owned()));
    };
    let url = text_of(file, FILE_ID)
        .or_else(|| text_of(file, FILE_DATA))
        .ok_or_else(|| HistoryError::Invalid("file needs a file_id or file_data".to_owned()))?;
    let hints = hints_of(file);
    let resolution = text_of(file, DETAIL).as_deref().and_then(convert_detail);
    let video_metadata = decode_video_metadata(file)?;
    let request = media_request!(
        &url,
        &hints,
        resolution,
        video_metadata.as_ref(),
        model,
        MediaField::File
    );
    push_media(parts, &request)
}

fn decode_video_metadata(fields: &Object) -> Result<Option<VideoMetadata>, HistoryError> {
    let Some(value) = fields
        .get(VIDEO_METADATA)
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(None);
    };
    let encoded = serde_json::to_value(value).map_err(HistoryError::Encode)?;
    let decoded = serde_json::from_value(encoded).map_err(HistoryError::Decode)?;
    Ok(Some(decoded))
}

pub fn append_assistant_parts(
    message: &Message,
    options: AssistantOptions<'_>,
    parts: &mut Vec<Object>,
    last_tool_calls: &mut Option<LastToolCallMessage>,
) -> Result<(), HistoryError> {
    let fields = &message.provider_specific_fields;
    if let Some(text) = text_of(fields, REASONING_CONTENT) {
        parts.push(object!("thought" => Json::Bool(true), TEXT => Json::Text(text)));
    }
    for block in blocks_of(fields) {
        let Some(entry) = block.as_object() else {
            continue;
        };
        if entry.get("type").and_then(Json::as_str) == Some("thinking")
            && let (Some(thinking), Some(signature)) = (
                entry.get("thinking").and_then(Json::as_str),
                entry.get("signature").and_then(Json::as_str),
            )
        {
            parts.push(decode_thinking_part(thinking, signature));
        }
    }
    append_content(message, fields, parts);
    append_images(fields, options.model, parts)?;
    append_tool_calls(message, fields, options, parts, last_tool_calls)?;
    append_invocations(fields, parts);
    Ok(())
}

fn blocks_of(fields: &Object) -> &[Json] {
    if let Some(Json::List(blocks)) = fields.get(THINKING_BLOCKS) {
        return blocks;
    }
    &[]
}

fn append_content(message: &Message, fields: &Object, parts: &mut Vec<Object>) {
    match &message.content {
        None => {}
        Some(Content::Text(text)) => {
            let mut part = object!(TEXT => Json::Text(text.clone()));
            if let Some(signature) = text_signature(fields) {
                part.insert("thoughtSignature".to_owned(), signature);
            }
            parts.push(part);
        }
        Some(Content::Parts(items)) => {
            for item in items {
                if item.kind.as_str() == TEXT
                    && let Some(text) = text_of(&item.fields, TEXT)
                {
                    parts.push(object!(TEXT => Json::Text(text)));
                }
            }
        }
    }
}

fn text_signature(fields: &Object) -> Option<Json> {
    let provider = fields.get(PROVIDER_FIELDS)?.as_object()?;
    let Some(Json::List(signatures)) = provider.get(THOUGHT_SIGNATURES) else {
        return None;
    };
    let signature = signatures.first()?.clone();
    let tool_signatures = collect_tool_signatures(fields);
    let unhashable = matches!(&signature, Json::List(_) | Json::Object(_));
    if unhashable || matches!(&signature, Json::Text(raw) if tool_signatures.contains(raw)) {
        return None;
    }
    Some(signature)
}

fn append_images(
    fields: &Object,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let Some(Json::List(items)) = fields.get(IMAGES) else {
        return Ok(());
    };
    for item in items {
        let Some(Json::Object(image)) = item.as_object().and_then(|entry| entry.get(IMAGE_URL))
        else {
            continue;
        };
        let Some(url) = text_of(image, URL).filter(|url| !url.is_empty()) else {
            continue;
        };
        let hints = hints_of(image);
        let resolution = text_of(image, DETAIL).as_deref().and_then(convert_detail);
        let request = media_request!(&url, &hints, resolution, None, model, MediaField::ImageUrl);
        push_media(parts, &request)?;
    }
    Ok(())
}
