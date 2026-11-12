use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use serde_json::Number;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AllDeltaType {
    Text,
    Audio,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeResponseTransformInput {
    pub session_configuration_request: Option<String>,
    pub current_output_item_id: Option<String>,
    pub current_response_id: Option<String>,
    pub current_delta_chunks: Option<Vec<Json>>,
    pub current_item_chunks: Option<Vec<Json>>,
    pub current_conversation_id: Option<String>,
    pub current_delta_type: Option<AllDeltaType>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeResponseTypedDict {
    pub response: Json,
    pub current_output_item_id: Option<String>,
    pub current_response_id: Option<String>,
    pub current_delta_chunks: Option<Vec<Json>>,
    pub current_conversation_id: Option<String>,
    pub current_item_chunks: Option<Vec<Json>>,
    pub current_delta_type: Option<AllDeltaType>,
    pub session_configuration_request: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorEventType {
    #[serde(rename = "error")]
    Error,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeErrorEvent {
    #[serde(rename = "type")]
    pub event_type: ErrorEventType,
    pub error: RealtimeErrorDetail,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputTokenDetails {
    pub text_tokens: i64,
    pub audio_tokens: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: InputTokenDetails,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationUsage {
    pub seconds: Number,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AudioTranscriptionUsage {
    Tokens(Box<TokenUsage>),
    Duration(DurationUsage),
}
