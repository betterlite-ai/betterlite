use serde_json::Value;

use crate::openai_video_identifiers::ids::rewrite_character_ids;
use crate::openai_video_identifiers::multipart::{MediaBody, MultipartPart, add_image_to_files};
use crate::videos::VideoOptions;
use crate::videos::error::VideoError;
use crate::videos::reply::{
    VideoObject, decode_video_reply, duration_usage, expect_video, wrap_video_id,
};

pub struct CreateVideoForm<'media> {
    pub fields: VideoOptions,
    pub files: Vec<MultipartPart<'media>>,
    pub api_base: String,
}

// transform_video_create_request: model, prompt and extra_headers never come from the options
#[must_use]
pub fn create_request<'media>(
    model: &str,
    prompt: &str,
    api_base: &str,
    options: &VideoOptions,
    input_reference: Option<MediaBody<'media>>,
) -> CreateVideoForm<'media> {
    let mut fields = VideoOptions::new();
    fields.insert("model".to_owned(), Value::String(model.to_owned()));
    fields.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
    for (name, value) in options {
        if matches!(
            name.as_str(),
            "model" | "prompt" | "extra_headers" | "input_reference"
        ) {
            continue;
        }
        fields.insert(name.clone(), value.clone());
    }
    rewrite_character_ids(&mut fields);
    let mut files = Vec::new();
    if let Some(reference) = input_reference {
        add_image_to_files(&mut files, "input_reference", reference);
    }
    CreateVideoForm {
        fields,
        files,
        api_base: api_base.to_owned(),
    }
}

// transform_video_create_response: a truthy id gains the provider and the requested model
pub fn create_response(
    model: &str,
    body: &[u8],
    provider: Option<&str>,
) -> Result<VideoObject, VideoError> {
    let mut video = expect_video(decode_video_reply(body)?)?;
    wrap_video_id(&mut video, provider, Some(model));
    video.usage = Some(duration_usage(&video));
    Ok(video)
}
