use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineData {
    pub mime_type: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub inline_data: InlineData,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub role: Role,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTranscriptionConfig {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub language_codes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationConfig {
    pub audio_transcription_config: AudioTranscriptionConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionRequest {
    pub contents: Vec<Content>,
    pub generation_config: GenerationConfig,
}
// LiteLLM's response models ignore unknown fields, and every tuple defaults to empty and every count to zero
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResponsePart {
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResponseContent {
    pub parts: Vec<ResponsePart>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Candidate {
    pub content: Option<ResponseContent>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModalityTokens {
    pub modality: Option<String>,
    pub token_count: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: i64,
    pub candidates_token_count: i64,
    pub total_token_count: i64,
    pub prompt_tokens_details: Vec<ModalityTokens>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResponseBody {
    pub candidates: Vec<Candidate>,
    pub usage_metadata: Option<UsageMetadata>,
}
