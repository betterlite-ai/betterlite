use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Blob {
    pub data: String,
    pub mime_type: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub file_uri: String,
}
// The media keys share one flat part map with the metadata keys, so the payload is optional here
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_data: Option<Blob>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<FileData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_resolution: Option<MediaResolution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_metadata: Option<VideoMetadata>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MediaResolution {
    pub level: MediaResolutionLevel,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MediaResolutionLevel {
    #[serde(rename = "MEDIA_RESOLUTION_LOW")]
    Low,
    #[serde(rename = "MEDIA_RESOLUTION_MEDIUM")]
    Medium,
    #[serde(rename = "MEDIA_RESOLUTION_HIGH")]
    High,
    #[serde(rename = "MEDIA_RESOLUTION_ULTRA_HIGH")]
    UltraHigh,
}
// The outer Option is an absent key, the inner one is an explicit null LiteLLM forwards
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VideoMetadata {
    #[serde(
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub fps: Option<Option<Json>>,
    #[serde(
        rename(serialize = "startOffset", deserialize = "start_offset"),
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_offset: Option<Option<Json>>,
    #[serde(
        rename(serialize = "endOffset", deserialize = "end_offset"),
        with = "serde_with::rust::double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_offset: Option<Option<Json>>,
}
