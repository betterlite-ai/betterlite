use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::VIDEO_FIELD;
use super::body::video_reference;
use crate::openai_video_identifiers::multipart::{MediaBody, MultipartPart, video_file_tuple};

pub struct VideoEditRequest<'media> {
    pub prompt: String,
    pub video_id: String,
    pub api_base: String,
    pub video_file: Option<MediaBody<'media>>,
    pub extra_body: Option<IndexMap<String, Json>>,
    // LiteLLM accepts a prefetched source and never reads it, so it can neither fetch media nor change the body
    pub prefetched_source_data: Option<Json>,
}

pub struct VideoEdit<'media> {
    pub url: String,
    pub body: IndexMap<String, Json>,
    pub files: Option<MultipartPart<'media>>,
}

#[must_use]
pub fn transform_video_edit_request<'media>(
    request: VideoEditRequest<'media>,
) -> VideoEdit<'media> {
    let url = format!("{}/edits", request.api_base.trim_end_matches('/'));
    let mut body = IndexMap::new();
    body.insert(String::from("prompt"), Json::Text(request.prompt));
    if request.video_file.is_none() {
        body.insert(String::from("video"), video_reference(&request.video_id));
    }
    body.extend(request.extra_body.unwrap_or_default());
    let files = request
        .video_file
        .map(|video| video_file_tuple(video, VIDEO_FIELD));
    VideoEdit { url, body, files }
}
