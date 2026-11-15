pub mod decode;
pub mod error;
pub mod params;
pub mod settings;
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::fal::images::decode::MAX_FAL_REPLY_BYTES;
use crate::api::fal::images::error::FalImageError;
use crate::api::fal::images::params::GENERIC_OPENAI_PARAMS;

#[derive(Debug, Error)]
pub enum UrlOnlyImageError {
    #[error("the size field must be a string")]
    SizeNotText,
    #[error("parameters {parameters:?} are not supported for model {model}")]
    UnsupportedParameters {
        parameters: Vec<String>,
        model: String,
    },
    #[error("the {field} field of a {model} image must be a string")]
    FieldNotText {
        field: &'static str,
        model: &'static str,
    },
    #[error(transparent)]
    Reply(#[from] FalImageError),
}

pub type SizeTranslation = fn(&Json) -> Result<(&'static str, Json), UrlOnlyImageError>;

// the pinned classes branch on the original option name, not the translated one, and overwrite the destination
pub fn map_url_only_params(
    non_default_params: &ImageParams,
    optional_params: ImageParams,
    model: &str,
    drop_params: bool,
    translate_size: SizeTranslation,
) -> Result<ImageParams, UrlOnlyImageError> {
    let mut mapped = optional_params;
    for (name, value) in non_default_params {
        if mapped.contains_key(name) {
            continue;
        }
        if !GENERIC_OPENAI_PARAMS.contains(&name.as_str()) {
            if drop_params {
                continue;
            }
            return Err(UrlOnlyImageError::UnsupportedParameters {
                parameters: vec![name.clone()],
                model: model.to_owned(),
            });
        }
        if name == "size" {
            let (destination, translated) = translate_size(value)?;
            mapped.insert(destination.to_owned(), translated);
        }
    }
    Ok(mapped)
}

pub fn decode_url_only_reply(
    status: u16,
    headers: &ImageHeaders,
    body: &[u8],
) -> Result<IndexMap<String, Json>, UrlOnlyImageError> {
    let value: Json = parse_reply(status, headers, body)?;
    let Json::Object(fields) = value else {
        return Err(UrlOnlyImageError::Reply(FalImageError::ReplyNotAnObject {
            status,
            headers: headers.clone(),
        }));
    };
    Ok(fields)
}

fn parse_reply(status: u16, headers: &ImageHeaders, body: &[u8]) -> Result<Json, FalImageError> {
    if body.len() > MAX_FAL_REPLY_BYTES {
        return Err(FalImageError::ReplyTooLarge {
            status,
            headers: headers.clone(),
            limit: MAX_FAL_REPLY_BYTES,
        });
    }
    serde_json::from_slice(body).map_err(|source| FalImageError::ReplyDecode {
        status,
        headers: headers.clone(),
        source,
    })
}

// B997 reads a missing or null field as none and refuses any other type, naming the field
pub fn image_url(
    image: &IndexMap<String, Json>,
    model: &'static str,
) -> Result<Option<String>, UrlOnlyImageError> {
    match image.get("url") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(url)) => Ok(Some(url.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(UrlOnlyImageError::FieldNotText {
                field: "url",
                model,
            })
        }
    }
}
