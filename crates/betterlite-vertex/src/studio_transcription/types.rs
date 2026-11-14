use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AudioInputType {
    #[serde(rename = "audio")]
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioInput {
    #[serde(rename = "type")]
    pub kind: AudioInputType,
    pub data: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TranscriptionGranularity {
    #[serde(rename = "word")]
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DiarizationMode {
    #[serde(rename = "speaker")]
    Speaker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TranscriptionModeType {
    #[serde(rename = "verbatim")]
    Verbatim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VerbatimMode {
    #[serde(rename = "type")]
    pub kind: TranscriptionModeType,
    pub timestamp_granularities: [TranscriptionGranularity; 1],
    pub diarization_mode: DiarizationMode,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TranscriptionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_codes: Option<[String; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<VerbatimMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationConfig {
    pub transcription_config: TranscriptionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InteractionRequest {
    pub model: String,
    pub input: [AudioInput; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WordAnnotation {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub text: Option<String>,
    pub speaker: Option<String>,
    pub start_offset: Option<String>,
    pub end_offset: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Content {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub text: Option<String>,
    #[serde(default)]
    pub annotations: Vec<WordAnnotation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub content: Vec<Content>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModalityTokens {
    pub modality: Option<String>,
    #[serde(default)]
    pub tokens: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub total_input_tokens: i64,
    #[serde(default)]
    pub total_output_tokens: i64,
    #[serde(default)]
    pub input_tokens_by_modality: Vec<ModalityTokens>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InteractionResponse {
    pub id: Option<String>,
    pub status: Option<String>,
    pub usage: Option<Usage>,
    #[serde(default)]
    pub steps: Vec<Step>,
}
