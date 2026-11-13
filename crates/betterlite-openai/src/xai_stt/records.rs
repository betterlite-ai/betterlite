use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// LiteLLM's _XAISttResponse and _XAISttWord keep xAI's extra keys, so every reply field not modelled here rides along
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SttPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub words: Option<Vec<SttWord>>,
    #[serde(flatten)]
    pub native_fields: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct SttWord {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
    #[serde(default)]
    pub speaker: Option<i64>,
    #[serde(flatten)]
    pub native_fields: Map<String, Value>,
}

fn default_language() -> String {
    "unknown".to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionTask {
    Transcribe,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptionWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<i64>,
}

// the complete native answer LiteLLM keeps for logging, plus the duration it copies out of it
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SttMetadata {
    pub text: String,
    pub language: String,
    pub duration: Option<f64>,
    pub words: Option<Vec<SttWord>>,
    #[serde(flatten)]
    pub native_fields: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_transcription_duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptionResponse {
    pub text: String,
    pub task: TranscriptionTask,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<TranscriptionWord>>,
    #[serde(skip)]
    pub metadata: SttMetadata,
}
