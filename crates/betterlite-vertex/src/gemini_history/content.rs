use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_invokes::types::GeminiPart;
use betterlite_prompts::gemini_tool_results::types::GeminiFunctionResponsePart;
use serde::Serialize;

use crate::gemini_history::error::HistoryError;
use crate::gemini_media::error::MediaField;
use crate::gemini_media::mime::MimeHints;
use crate::gemini_media::part::{MediaPart, MediaResolution, VideoMetadata};
use crate::gemini_media::process::{MediaRequest, process_media};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Model,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Content {
    pub role: Role,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Part {
    Text(TextPart),
    Thought(ThoughtPart),
    Media(MediaPart),
    FunctionCall(GeminiPart),
    FunctionResponse(GeminiFunctionResponsePart),
    ServerToolCall(ServerToolCallPart),
    ServerToolResponse(ServerToolResponsePart),
    Decoded(Object),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextPart {
    pub text: String,
    // A null entry in thought_signatures is replayed as an explicit null signature
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none")]
    pub signature: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThoughtPart {
    pub thought: bool,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerToolCallPart {
    #[serde(rename = "toolCall")]
    pub tool_call: ServerToolCall,
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none")]
    pub signature: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerToolCall {
    #[serde(rename = "toolType")]
    pub tool_type: Json,
    pub id: Json,
    pub args: Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerToolResponsePart {
    #[serde(rename = "toolResponse")]
    pub tool_response: ServerToolResponse,
    #[serde(rename = "thoughtSignature", skip_serializing_if = "Option::is_none")]
    pub signature: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerToolResponse {
    pub id: Json,
    pub response: Json,
    #[serde(rename = "toolType", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<Json>,
}

pub struct MediaInput<'media> {
    pub url: &'media str,
    pub hints: &'media MimeHints,
    pub resolution: Option<MediaResolution>,
    pub video_metadata: Option<&'media VideoMetadata>,
    pub model: Option<&'media str>,
    pub field: MediaField,
}

pub fn push_media(parts: &mut Vec<Part>, media: &MediaInput<'_>) -> Result<(), HistoryError> {
    let request = MediaRequest {
        url: media.url,
        hints: media.hints,
        resolution: media.resolution,
        video_metadata: media.video_metadata,
        model: media.model,
        field: media.field,
    };
    let part = process_media(&request)?;
    parts.push(Part::Media(part));
    Ok(())
}

pub fn part_object(part: &Part) -> Result<Object, HistoryError> {
    let encoded = serde_json::to_value(part).map_err(|source| HistoryError::Encode { source })?;
    let decoded = serde_json::from_value::<Json>(encoded)
        .map_err(|source| HistoryError::Encode { source })?;
    match decoded {
        Json::Object(object) => Ok(object),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(HistoryError::PartShape)
        }
    }
}
