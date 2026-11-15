use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::body::video_reference;

pub struct VideoExtensionRequest {
    pub prompt: String,
    pub video_id: String,
    // LiteLLM types seconds as a string and passes whatever it was given through unchanged
    pub seconds: String,
    pub api_base: String,
    pub extra_body: Option<IndexMap<String, Json>>,
}

pub struct VideoExtension {
    pub url: String,
    pub body: IndexMap<String, Json>,
}

#[must_use]
pub fn transform_video_extension_request(request: VideoExtensionRequest) -> VideoExtension {
    let mut body = IndexMap::new();
    body.insert(String::from("prompt"), Json::Text(request.prompt));
    body.insert(String::from("seconds"), Json::Text(request.seconds));
    body.insert(String::from("video"), video_reference(&request.video_id));
    body.extend(request.extra_body.unwrap_or_default());
    VideoExtension {
        url: format!("{}/extensions", request.api_base.trim_end_matches('/')),
        body,
    }
}
