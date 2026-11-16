use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use serde::Serialize;
use serde_json::Number;

use super::VideoParams;
use super::decode::FalVideoReply;
use super::duration::numeric_duration;
use super::error::FalVideoError;
use super::profile::profile_for_model;

pub const FAL_AI_PROVIDER: &str = "fal_ai";

pub struct CreateResponseInput<'a> {
    pub model: &'a str,
    pub reply: &'a FalVideoReply,
    pub custom_llm_provider: Option<&'a str>,
    pub request_data: &'a VideoParams,
    pub created_at: i64,
}

#[derive(Serialize)]
struct CreateWire<'a> {
    id: String,
    object: VideoObjectType,
    status: &'a str,
    created_at: i64,
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<&'a str>,
    usage: UsageWire,
}

#[derive(Serialize)]
struct UsageWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<Number>,
    video_resolution: String,
}

#[must_use]
pub fn create_request(
    model: &str,
    prompt: &str,
    api_base: &str,
    options: &VideoParams,
) -> (VideoParams, String) {
    let mut data = VideoParams::new();
    data.insert("prompt".to_owned(), Json::Text(prompt.to_owned()));
    for (key, value) in options {
        if key != "model" {
            data.insert(key.clone(), value.clone());
        }
    }
    (data, format!("{}/{model}", api_base.trim_end_matches('/')))
}

pub fn create_response(input: &CreateResponseInput<'_>) -> Result<VideoObject, FalVideoError> {
    let profile = profile_for_model(input.model);
    let provider = input
        .custom_llm_provider
        .filter(|provider| !provider.is_empty())
        .unwrap_or(FAL_AI_PROVIDER);
    let request_id = input.reply.request_id.as_deref().unwrap_or_default();
    let duration = numeric_duration(input.request_data.get("duration"));
    let resolution = resolution_text(input.request_data.get("resolution"));
    let usage = UsageWire {
        duration_seconds: match &duration {
            Some(text) => Some(usage_number(text)?),
            None => None,
        },
        video_resolution: resolution.unwrap_or(profile.default_resolution).to_owned(),
    };
    let wire = CreateWire {
        id: encode_video_id_with_provider(request_id, provider, Some(input.model)),
        object: VideoObjectType::Video,
        status: "queued",
        created_at: input.created_at,
        model: input.model,
        seconds: duration.as_deref(),
        size: resolution,
        usage,
    };
    let value = serde_json::to_value(wire).map_err(FalVideoError::VideoObjectEncode)?;
    serde_json::from_value::<VideoObject>(value).map_err(FalVideoError::VideoObjectDecode)
}

fn resolution_text(resolution: Option<&Json>) -> Option<&str> {
    match resolution {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn usage_number(text: &str) -> Result<Number, FalVideoError> {
    format!("{text}.0")
        .parse::<Number>()
        .map_err(FalVideoError::VideoObjectEncode)
}
