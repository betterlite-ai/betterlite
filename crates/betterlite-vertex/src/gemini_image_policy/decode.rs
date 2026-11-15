use std::str::from_utf8;

use betterlite_prompts::arguments::json::{Json, Object};

use super::error::GeminiImagePolicyError;

pub const MAX_IMAGE_OPTIONS_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageOptions {
    Size(String),
    Config(Object),
}

pub fn decode_image_options(bytes: &[u8]) -> Result<ImageOptions, GeminiImagePolicyError> {
    if bytes.len() > MAX_IMAGE_OPTIONS_BYTES {
        return Err(GeminiImagePolicyError::OptionsTooLarge {
            limit: MAX_IMAGE_OPTIONS_BYTES,
        });
    }
    let text = from_utf8(bytes).map_err(GeminiImagePolicyError::OptionsNotUtf8)?;
    let trimmed = text.trim();
    if !trimmed.starts_with(['{', '[', '"']) {
        return Ok(ImageOptions::Size(trimmed.to_owned()));
    }
    let decoded =
        serde_json::from_str::<Json>(trimmed).map_err(GeminiImagePolicyError::OptionsJson)?;
    match decoded {
        Json::Object(object) => Ok(ImageOptions::Config(object)),
        Json::Text(size) => Ok(ImageOptions::Size(size)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            Err(GeminiImagePolicyError::OptionsShape)
        }
    }
}
