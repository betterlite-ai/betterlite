use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::openai_video_identifiers::ids::rewrite_character_ids;
use crate::openai_video_identifiers::multipart::{MediaBody, MultipartPart, add_image_to_files};
use crate::videos::VideoOptions;
use crate::videos::config::map_params;
use crate::videos::error::VideoError;
use crate::videos::reply::{
    VideoObject, decode_video_reply, duration_usage, expect_video, wrap_video_id,
};

// VideoCreateOptionalRequestParams: the declared text options are typed, every other key rides along
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct VideoCreate {
    pub seconds: Option<String>,
    pub size: Option<String>,
    pub user: Option<String>,
    pub characters: Option<Vec<BTreeMap<String, String>>>,
    #[serde(flatten)]
    pub extra: VideoOptions,
}

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
    options: &VideoCreate,
    input_reference: Option<MediaBody<'media>>,
) -> CreateVideoForm<'media> {
    let mut fields = VideoOptions::new();
    map_params(options, &mut fields);
    for name in ["model", "prompt", "extra_headers", "input_reference"] {
        fields.remove(name);
    }
    fields.insert("model".to_owned(), Value::String(model.to_owned()));
    fields.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
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
