use betterlite_models::decisions::Json;

use super::{DEFAULT_API_BASE, VideoParams};

const TEXT_TO_VIDEO: &str = "text_to_video";
const IMAGE_TO_VIDEO: &str = "image_to_video";
const VIDEO_TO_VIDEO: &str = "video_to_video";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationEndpoint {
    TextToVideo,
    ImageToVideo,
    VideoToVideo,
}

impl GenerationEndpoint {
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::TextToVideo => TEXT_TO_VIDEO,
            Self::ImageToVideo => IMAGE_TO_VIDEO,
            Self::VideoToVideo => VIDEO_TO_VIDEO,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateVideoForm {
    pub body: VideoParams,
    pub url: String,
}

// transform_video_create_request: model and promptText first, then every caller field, then the endpoint the merged body selects
#[must_use]
pub fn create_request(
    model: &str,
    prompt: &str,
    api_base: Option<&str>,
    params: &VideoParams,
) -> CreateVideoForm {
    let mut body = VideoParams::new();
    body.insert("model".to_owned(), Json::Text(model.to_owned()));
    body.insert("promptText".to_owned(), Json::Text(prompt.to_owned()));
    for (name, value) in params {
        body.insert(name.clone(), value.clone());
    }
    let non_null =
        |name: &str| matches!(body.get(name), Some(value) if !matches!(value, Json::Null));
    let endpoint = if non_null("promptVideo") || non_null("videoUri") {
        GenerationEndpoint::VideoToVideo
    } else if non_null("promptImage") {
        GenerationEndpoint::ImageToVideo
    } else {
        GenerationEndpoint::TextToVideo
    };
    if endpoint != GenerationEndpoint::ImageToVideo {
        body.shift_remove("promptImage");
    }
    let base = api_base.unwrap_or(DEFAULT_API_BASE).trim_end_matches('/');
    CreateVideoForm {
        body,
        url: format!("{base}/{}", endpoint.path()),
    }
}
