use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RealtimeExpiresAfter {
    pub anchor: Option<String>,
    pub seconds: Option<i64>,
}
impl Default for RealtimeExpiresAfter {
    fn default() -> Self {
        Self {
            anchor: Some("created_at".to_owned()),
            seconds: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaxOutputTokens {
    Number(Number),
    Text(String),
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeSessionConfig {
    #[serde(rename = "type")]
    pub session_type: Option<String>,
    pub model: Option<String>,
    pub instructions: Option<String>,
    pub audio: Option<IndexMap<String, Json>>,
    pub include: Option<Vec<String>>,
    pub max_output_tokens: Option<MaxOutputTokens>,
    pub output_modalities: Option<Vec<String>>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<IndexMap<String, Json>>>,
    pub tracing: Option<Json>,
    pub truncation: Option<Json>,
    pub prompt: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeClientSecretRequest {
    pub expires_after: Option<RealtimeExpiresAfter>,
    pub session: Option<RealtimeSessionConfig>,
    pub model: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeClientSecretResponse {
    pub expires_at: Option<i64>,
    pub value: String,
    pub session: Option<IndexMap<String, Json>>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionSessionRequest {
    pub model: Option<String>,
    pub input_audio_transcription: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}
impl TranscriptionSessionRequest {
    #[must_use]
    pub fn resolved_model(&self) -> Option<&str> {
        if let Some(model) = self.model.as_deref()
            && !model.is_empty()
        {
            return Some(model);
        }
        match self.input_audio_transcription.as_ref()?.get("model")? {
            Json::Text(nested) => Some(nested.as_str()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptionSessionResponse {
    pub client_secret: Option<IndexMap<String, Json>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}
