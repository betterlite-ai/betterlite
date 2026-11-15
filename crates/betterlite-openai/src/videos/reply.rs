use std::num::FpCategory;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::error::VideoError;
use crate::videos::list::{VideoList, carries_list_data};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum VideoObjectType {
    #[serde(rename = "video")]
    Video,
}

// VideoObject: the fields LiteLLM's pydantic model declares, in the same optionality
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct VideoObject {
    pub id: String,
    pub object: VideoObjectType,
    pub status: String,
    pub created_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub error: Option<Map<String, Value>>,
    pub progress: Option<i64>,
    pub remixed_from_video_id: Option<String>,
    pub seconds: Option<Value>,
    pub size: Option<String>,
    pub model: Option<String>,
    pub usage: Option<Map<String, Value>>,
}

impl VideoObject {
    // float(seconds) in LiteLLM: a string "0" is truthy and becomes 0.0, a numeric 0 is not
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        match self.seconds.as_ref()? {
            Value::String(text) => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|parsed| parsed.is_finite()),
            Value::Number(number) => number
                .as_f64()
                .filter(|value| value.classify() != FpCategory::Zero && value.is_finite()),
            Value::Bool(true) => Some(1.0),
            Value::Bool(false) | Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }
}

// the encoder leaves an empty provider or id alone; only create passes a model
pub(crate) fn wrap_video_id(video: &mut VideoObject, provider: Option<&str>, model: Option<&str>) {
    if let Some(name) = provider {
        video.id = encode_video_id_with_provider(&video.id, name, model);
    }
}

#[must_use]
pub fn duration_usage(video: &VideoObject) -> Map<String, Value> {
    let mut usage = Map::new();
    if let Some(seconds) = video.duration_seconds() {
        usage.insert("duration_seconds".to_owned(), Value::from(seconds));
    }
    usage
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoReply {
    Video(Box<VideoObject>),
    List(Box<VideoList>),
}

// decode_video_reply: LiteLLM picks the shape per endpoint, so the data key is the list marker
pub fn decode_video_reply(body: &[u8]) -> Result<VideoReply, VideoError> {
    let json = serde_json::from_slice::<Value>(body)?;
    if carries_list_data(&json) {
        let list = serde_json::from_value::<VideoList>(json)?;
        return Ok(VideoReply::List(Box::new(list)));
    }
    let video = serde_json::from_value::<VideoObject>(json)?;
    Ok(VideoReply::Video(Box::new(video)))
}

// the video endpoints (create, remix, status, delete) accept only the single-object shape
pub fn decode_video(body: &[u8]) -> Result<VideoObject, VideoError> {
    match decode_video_reply(body)? {
        VideoReply::Video(video) => Ok(*video),
        VideoReply::List(_) => Err(VideoError::ExpectedVideo),
    }
}
