use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::VideoEditError;
use crate::openai_video_identifiers::ids::encode_video_id_with_provider;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterKind {
    #[default]
    Character,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterObject {
    pub id: String,
    #[serde(default)]
    pub object: CharacterKind,
    pub created_at: i64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoKind {
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoObject {
    pub id: String,
    pub object: VideoKind,
    pub status: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub error: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub progress: Option<i64>,
    #[serde(default)]
    pub remixed_from_video_id: Option<String>,
    #[serde(default)]
    pub seconds: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<IndexMap<String, Json>>,
}

// ast-grep-ignore: no-one-line-functions - the task names this decoder as the fuzz target's entry point
pub fn decode_character_reply(bytes: &[u8]) -> Result<CharacterObject, VideoEditError> {
    serde_json::from_slice(bytes).map_err(VideoEditError::CharacterReply)
}

pub struct VideoReply<'reply> {
    pub body: &'reply [u8],
    // LiteLLM wraps the id only when this is truthy, so an empty provider leaves the id as sent
    pub custom_llm_provider: Option<&'reply str>,
    // LiteLLM accepts the request data on an edit reply and never reads it
    pub request_data: Option<&'reply Json>,
}

pub fn transform_video_response(reply: &VideoReply<'_>) -> Result<VideoObject, VideoEditError> {
    let mut object: VideoObject =
        serde_json::from_slice(reply.body).map_err(VideoEditError::VideoReply)?;
    if let Some(provider) = reply.custom_llm_provider
        && !provider.is_empty()
        && !object.id.is_empty()
    {
        object.id = encode_video_id_with_provider(&object.id, provider, None);
    }
    Ok(object)
}
