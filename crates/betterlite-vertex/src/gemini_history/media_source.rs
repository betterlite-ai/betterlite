use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::ContentPart;

use crate::gemini_history::content::{CONTENT_TYPE, DETAIL, FORMAT, MIME_TYPE, object, push_media};
use crate::gemini_history::error::HistoryError;
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::MimeHints;
use crate::gemini_media::part::VideoMetadata;
use crate::gemini_media::process::MediaRequest;
use crate::gemini_media::resolution::convert_detail;

const IMAGE_URL: &str = "image_url";
const INPUT_AUDIO: &str = "input_audio";
const FILE: &str = "file";
const TEXT: &str = "text";
const URL: &str = "url";
const DATA: &str = "data";
const FILE_ID: &str = "file_id";
const FILE_DATA: &str = "file_data";
const VIDEO_METADATA: &str = "video_metadata";
const VIDEO_KEYS: [&str; 3] = ["fps", "start_offset", "end_offset"];

pub type Locator = (String, Object);

#[must_use]
pub fn text_of(fields: &Object, name: &str) -> Option<String> {
    if let Some(Json::Text(value)) = fields.get(name) {
        return Some(value.clone());
    }
    None
}

fn hints_of(fields: &Object) -> MimeHints {
    MimeHints {
        format: text_of(fields, FORMAT),
        mime_type: text_of(fields, MIME_TYPE),
        content_type: text_of(fields, CONTENT_TYPE),
    }
}

pub fn push_source(
    parts: &mut Vec<Object>,
    locator: &Locator,
    field: MediaField,
    model: Option<&str>,
) -> Result<(), HistoryError> {
    let (url, fields) = locator;
    let resolution = text_of(fields, DETAIL).as_deref().and_then(convert_detail);
    let video = if field == MediaField::File {
        decode_video(fields)?
    } else {
        None
    };
    let request = MediaRequest {
        url,
        hints: &hints_of(fields),
        resolution,
        video_metadata: video.as_ref(),
        model,
        field,
    };
    push_media(parts, &request).map_err(|error| {
        let HistoryError::Media(source) = error else {
            return error;
        };
        let unusable = matches!(
            source,
            MediaError::InvalidMedia(_) | MediaError::InvalidInlineData(_)
        );
        if !unusable || field != MediaField::File {
            return HistoryError::Media(source);
        }
        let named = text_of(fields, FILE_ID).unwrap_or_else(|| "provided data".to_owned());
        HistoryError::FileMedia { named, source }
    })
}

fn image_source(fields: &Object) -> Result<Locator, HistoryError> {
    match fields.get(IMAGE_URL) {
        Some(Json::Text(url)) => Ok((url.clone(), Object::default())),
        Some(Json::Object(image)) => {
            let url = text_of(image, URL)
                .ok_or_else(|| HistoryError::Invalid("image_url.url is missing".to_owned()))?;
            Ok((url, image.clone()))
        }
        None | Some(Json::Null) => Err(HistoryError::Invalid("image_url is missing".to_owned())),
        Some(_) => Err(HistoryError::Invalid(
            "image_url must be a string or an object".to_owned(),
        )),
    }
}

fn required<'fields>(fields: &'fields Object, name: &str) -> Result<&'fields Object, HistoryError> {
    if let Some(Json::Object(found)) = fields.get(name) {
        return Ok(found);
    }
    Err(HistoryError::Invalid(format!("{name} is missing")))
}

fn audio_source(fields: &Object) -> Result<Option<Locator>, HistoryError> {
    let audio = required(fields, INPUT_AUDIO)?;
    let (Some(data), Some(format)) = (text_of(audio, DATA), text_of(audio, FORMAT)) else {
        return Ok(None);
    };
    let media_type = if format.starts_with("audio/") {
        format
    } else {
        format!("audio/{format}")
    };
    let url = format!("data:{media_type};base64,{data}");
    Ok(Some((url, object!(FORMAT => Json::Text(media_type)))))
}

fn file_source(fields: &Object) -> Result<Locator, HistoryError> {
    let file = required(fields, FILE)?;
    let url = text_of(file, FILE_ID)
        .or_else(|| text_of(file, FILE_DATA))
        .ok_or_else(|| HistoryError::Invalid("file needs a file_id or file_data".to_owned()))?;
    Ok((url, file.clone()))
}

fn decode_video(file: &Object) -> Result<Option<VideoMetadata>, HistoryError> {
    let metadata = match file.get(VIDEO_METADATA) {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Object(metadata)) => metadata,
        Some(_) => {
            return Err(HistoryError::Invalid(
                "video_metadata must be an object".to_owned(),
            ));
        }
    };
    let mut kept = Object::default();
    for key in VIDEO_KEYS {
        if let Some(value) = metadata.get(key) {
            kept.insert(key.to_owned(), value.clone());
        }
    }
    let encoded = serde_json::to_value(Json::Object(kept)).map_err(HistoryError::Encode)?;
    Ok(Some(
        serde_json::from_value(encoded).map_err(HistoryError::Decode)?,
    ))
}

#[must_use]
pub fn generated_image(item: &Json) -> Option<Locator> {
    let image = item.as_object()?.get(IMAGE_URL)?.as_object()?;
    let url = text_of(image, URL).filter(|url| !url.is_empty())?;
    Some((url, image.clone()))
}

pub fn append_user_part(
    item: &ContentPart,
    model: Option<&str>,
    parts: &mut Vec<Object>,
) -> Result<(), HistoryError> {
    let fields = &item.fields;
    let (locator, field) = match item.kind.as_str() {
        TEXT => {
            if let Some(text) = text_of(fields, TEXT)
                && !text.is_empty()
            {
                parts.push(object!(TEXT => Json::Text(text)));
            }
            return Ok(());
        }
        IMAGE_URL => (image_source(fields)?, MediaField::ImageUrl),
        INPUT_AUDIO => match audio_source(fields)? {
            Some(locator) => (locator, MediaField::ImageUrl),
            None => return Ok(()),
        },
        FILE => (file_source(fields)?, MediaField::File),
        _ => return Ok(()),
    };
    push_source(parts, &locator, field, model)
}
