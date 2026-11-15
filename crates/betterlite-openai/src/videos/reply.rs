use std::num::FpCategory;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::openai_video_identifiers::ids::encode_video_id_with_provider;
use crate::videos::error::VideoError;
use crate::videos::list::{VideoList, carries_list_data, decode_list};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum VideoObjectType {
    #[serde(rename = "video")]
    Video,
}

// VideoObject: the fields LiteLLM's pydantic model declares, in the same optionality
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
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

fn parse_duration(text: &str) -> Option<f64> {
    let parsed: f64 = text.trim().parse().ok()?;
    parsed.is_finite().then_some(parsed)
}

impl VideoObject {
    // float(seconds) in LiteLLM: a string "0" is truthy and becomes 0.0, a numeric 0 is not
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        match self.seconds.as_ref()? {
            Value::String(text) => parse_duration(text),
            Value::Number(number) => number
                .as_f64()
                .filter(|value| value.classify() != FpCategory::Zero && value.is_finite()),
            Value::Bool(true) => Some(1.0),
            Value::Bool(false) | Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }
}

// a truthy provider and id take the provider into the id; only create passes a model
pub(crate) fn wrap_video_id(video: &mut VideoObject, provider: Option<&str>, model: Option<&str>) {
    if let Some(provider) = provider.filter(|value| !value.is_empty())
        && !video.id.is_empty()
    {
        video.id = encode_video_id_with_provider(&video.id, provider, model);
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

#[derive(Clone, Debug, PartialEq)]
pub enum VideoReply {
    Video(Box<VideoObject>),
    List(Box<VideoList>),
}

pub(crate) fn decode_value(body: &[u8]) -> Result<Value, VideoError> {
    serde_json::from_slice(body).map_err(VideoError::from)
}

// decode_video_reply: LiteLLM picks the shape per endpoint, so the data key is the list marker
pub fn decode_video_reply(body: &[u8]) -> Result<VideoReply, VideoError> {
    let value = decode_value(body)?;
    if carries_list_data(&value) {
        let list = decode_list(value)?;
        return Ok(VideoReply::List(Box::new(list)));
    }
    serde_json::from_value(value)
        .map(|video| VideoReply::Video(Box::new(video)))
        .map_err(VideoError::from)
}

pub(crate) fn expect_video(reply: VideoReply) -> Result<VideoObject, VideoError> {
    match reply {
        VideoReply::Video(video) => Ok(*video),
        VideoReply::List(_) => Err(VideoError::ExpectedVideo),
    }
}
