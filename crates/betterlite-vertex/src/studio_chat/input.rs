use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};

use crate::gemini_history::error::HistoryError;
use crate::gemini_history::media_source::text_of;
use crate::gemini_media::error::{MediaError, MediaField};
use crate::gemini_media::mime::MimeHints;
use crate::gemini_media::policy::{GoogleProvider, inlines_remote};

const IMAGE_URL_TYPE: &str = "image_url";
const FILE_TYPE: &str = "file";
const IMAGE_URL: &str = "image_url";
const FILE: &str = "file";
const FILE_ID: &str = "file_id";
const URL: &str = "url";
const FORMAT: &str = "format";
const DETAIL: &str = "detail";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageUrlFields {
    pub url: Option<String>,
    pub format: Option<String>,
    pub detail: Option<String>,
}

// LiteLLM reads the key off the whole part: an object carries url, format and detail, a bare string only the url
#[must_use]
pub fn image_url_fields(part: &Object) -> ImageUrlFields {
    match part.get(IMAGE_URL) {
        Some(Json::Object(image)) => ImageUrlFields {
            url: text_of(image, URL),
            format: text_of(image, FORMAT),
            detail: text_of(image, DETAIL),
        },
        Some(Json::Text(url)) => ImageUrlFields {
            url: Some(url.clone()),
            format: None,
            detail: None,
        },
        None | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            ImageUrlFields::default()
        }
    }
}

// Studio downloads every remote URL outside the Files API before it converts the message, so a request here fails instead
pub(crate) fn reject_remote_media(messages: &[Message]) -> Result<(), HistoryError> {
    for message in messages {
        let Some(Content::Parts(parts)) = message.content.as_ref() else {
            continue;
        };
        for part in parts {
            match part.kind.as_str() {
                IMAGE_URL_TYPE => reject_image_url(&part.fields)?,
                FILE_TYPE => reject_file(&part.fields)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn reject_image_url(fields: &Object) -> Result<(), HistoryError> {
    let Some(url) = image_url_fields(fields).url else {
        return Ok(());
    };
    if inlines_remote(GoogleProvider::AiStudio, &url, &MimeHints::default()) {
        return Err(HistoryError::Media(MediaError::RemoteMediaRequired(
            MediaField::ImageUrl,
        )));
    }
    Ok(())
}

fn reject_file(fields: &Object) -> Result<(), HistoryError> {
    let file = match fields.get(FILE) {
        Some(Json::Object(file)) => file,
        None | Some(Json::Null) => {
            return Err(HistoryError::Invalid("file is missing".to_owned()));
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(HistoryError::Invalid("file must be an object".to_owned()));
        }
    };
    let Some(file_id) = text_of(file, FILE_ID) else {
        return Ok(());
    };
    if inlines_remote(GoogleProvider::AiStudio, &file_id, &MimeHints::default()) {
        return Err(HistoryError::Media(MediaError::RemoteMediaRequired(
            MediaField::File,
        )));
    }
    Ok(())
}
