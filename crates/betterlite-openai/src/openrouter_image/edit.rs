use std::io::SeekFrom;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::decode::decode_openrouter_image_reply;
use super::shared::{
    CHAT_COMPLETIONS, DEFAULT_API_BASE, ImageConfigError, ImageHiddenParams, SKIPPED_PARAMS,
    SUPPORTED_PARAMS, apply_usage, authorization, map_quality_to_image_size,
    map_size_to_aspect_ratio, reply_error, resolved_base, resolved_key, transform_error,
};
use crate::api::image_contracts::records::{
    ImageFiles, ImageHttpResponse, ImageParams, ImageResponse, ImageUsage,
};
use crate::openai_base_config::Headers;
use crate::openai_video_identifiers::multipart::{MediaBody, ReadSeek, image_content_type};
use crate::openrouter_chat::error::OpenRouterError;

const STAGE: &str = "image edit";
pub const MULTIPART_FORM_DATA: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenRouterImageEditConfig {
    pub supported_params: [&'static str; 3],
}

#[must_use]
// ast-grep-ignore: no-long-names
pub const fn get_openrouter_image_edit_config(_model: &str) -> OpenRouterImageEditConfig {
    OpenRouterImageEditConfig {
        supported_params: SUPPORTED_PARAMS,
    }
}

pub enum SourceImage<'image> {
    Bytes(&'image [u8]),
    Reader(&'image mut dyn ReadSeek),
    Unsupported,
}

impl OpenRouterImageEditConfig {
    pub fn map_params(&self, supplied: &ImageParams) -> Result<ImageParams, ImageConfigError> {
        let mut mapped = ImageParams::new();
        let mut config = IndexMap::new();
        for (name, value) in supplied {
            if !self.supported_params.contains(&name.as_str()) {
                continue;
            }
            let changed = map_edit_param(name.as_str(), value, &mut mapped, &mut config)?;
            if changed {
                mapped.insert(String::from("image_config"), Json::Object(config.clone()));
            }
        }
        Ok(mapped)
    }
}

fn map_edit_param(
    name: &str,
    value: &Json,
    mapped: &mut ImageParams,
    config: &mut IndexMap<String, Json>,
) -> Result<bool, ImageConfigError> {
    match name {
        "size" => {
            let Json::Text(size) = value else {
                return Err(ImageConfigError::Size);
            };
            config.insert(
                String::from("aspect_ratio"),
                Json::Text(map_size_to_aspect_ratio(size).to_owned()),
            );
            Ok(true)
        }
        "quality" => {
            let Json::Text(quality) = value else {
                return Err(ImageConfigError::Quality);
            };
            if let Some(image_size) = map_quality_to_image_size(quality) {
                config.insert(
                    String::from("image_size"),
                    Json::Text(image_size.to_owned()),
                );
                return Ok(true);
            }
            Ok(false)
        }
        _ => {
            mapped.insert(String::from(name), value.clone());
            Ok(false)
        }
    }
}

#[must_use]
pub fn complete_url(api_base: Option<&str>, environment_api_base: Option<&str>) -> String {
    let Some(base) = resolved_base([api_base, environment_api_base]) else {
        return format!("{DEFAULT_API_BASE}{CHAT_COMPLETIONS}");
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(CHAT_COMPLETIONS) {
        return trimmed.to_owned();
    }
    format!("{trimmed}{CHAT_COMPLETIONS}")
}

pub fn validate_environment(
    mut headers: Headers,
    api_key: Option<&SecretString>,
    global_api_key: Option<&SecretString>,
    environment_key: Option<&SecretString>,
) -> Result<Headers, ImageConfigError> {
    let key = resolved_key([api_key, global_api_key, environment_key])
        .ok_or(ImageConfigError::MissingApiKey)?;
    headers.insert(String::from("Authorization"), authorization(Some(key)));
    Ok(headers)
}

pub fn request_body(
    model: &str,
    prompt: Option<&str>,
    images: &mut [Option<SourceImage<'_>>],
    optional_params: &ImageParams,
) -> Result<(ImageParams, ImageFiles), ImageConfigError> {
    let mut content = Vec::new();
    for image in images.iter_mut().flatten() {
        content.push(image_part(image_data_url(image)?));
    }
    if let Some(prompt) = prompt
        && !prompt.is_empty()
    {
        content.push(prompt_part(prompt));
    }
    let mut body = ImageParams::new();
    body.insert(String::from("model"), Json::Text(model.to_owned()));
    body.insert(
        String::from("messages"),
        Json::List(vec![user_message(content)]),
    );
    body.insert(
        String::from("modalities"),
        Json::List(vec![
            Json::Text(String::from("image")),
            Json::Text(String::from("text")),
        ]),
    );
    for (name, value) in optional_params {
        if !SKIPPED_PARAMS.contains(&name.as_str()) {
            body.insert(name.clone(), value.clone());
        }
    }
    Ok((body, ImageFiles::new()))
}

fn image_data_url(image: &mut SourceImage<'_>) -> Result<String, ImageConfigError> {
    let bytes = match image {
        SourceImage::Bytes(bytes) => *bytes,
        SourceImage::Reader(reader) => return reader_data_url(&mut **reader),
        SourceImage::Unsupported => return Err(ImageConfigError::UnsupportedImage),
    };
    let mime = image_content_type(&mut MediaBody::Bytes(bytes));
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn reader_data_url(reader: &mut dyn ReadSeek) -> Result<String, ImageConfigError> {
    let position = reader
        .stream_position()
        .map_err(|source| ImageConfigError::ImageRead { source })?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|source| ImageConfigError::ImageRead { source })?;
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| ImageConfigError::ImageRead { source })?;
    reader
        .seek(SeekFrom::Start(position))
        .map_err(|source| ImageConfigError::ImageRead { source })?;
    let mime = image_content_type(&mut MediaBody::Bytes(&bytes));
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(&bytes)))
}

fn image_part(url: String) -> Json {
    Json::Object(IndexMap::from([
        (String::from("type"), Json::Text(String::from("image_url"))),
        (
            String::from("image_url"),
            Json::Object(IndexMap::from([(String::from("url"), Json::Text(url))])),
        ),
    ]))
}

fn prompt_part(prompt: &str) -> Json {
    Json::Object(IndexMap::from([
        (String::from("type"), Json::Text(String::from("text"))),
        (String::from("text"), Json::Text(prompt.to_owned())),
    ]))
}

fn user_message(content: Vec<Json>) -> Json {
    Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(String::from("user"))),
        (String::from("content"), Json::List(content)),
    ]))
}

pub fn transform_response(
    raw: &ImageHttpResponse,
    created: i64,
    model: &str,
) -> Result<(ImageResponse, ImageHiddenParams), OpenRouterError> {
    let reply =
        decode_openrouter_image_reply(&raw.body).map_err(|error| reply_error(error, raw, STAGE))?;
    let mut response = ImageResponse {
        created,
        data: Some(reply.images),
        usage: Some(ImageUsage::default()),
    };
    let mut hidden = ImageHiddenParams::default();
    if let Some(usage) = &reply.usage {
        apply_usage(usage, &mut response, &mut hidden, usage.input_image_tokens)
            .map_err(|error| transform_error(STAGE, &error.to_string()))?;
    }
    hidden.model = Some(reply.model.unwrap_or_else(|| Json::Text(model.to_owned())));
    Ok((response, hidden))
}
