use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionFeatures {
    pub enable_automatic_punctuation: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionConfig {
    pub model: String,
    pub language_codes: Vec<String>,
    pub features: RecognitionFeatures,
    pub auto_decoding_config: Object, // LiteLLM's empty TypedDict, so {} on the wire
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecognizeRequest {
    pub config: RecognitionConfig,
    pub content: String,
}
// pydantic ignores fields it does not model, so every record keeps the rest of the provider's envelope
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Alternative {
    pub transcript: Option<String>,
    #[serde(flatten)]
    pub native_fields: Object,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecognitionResult {
    pub alternatives: Vec<Alternative>,
    pub language_code: Option<String>,
    #[serde(flatten)]
    pub native_fields: Object,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResponseMetadata {
    pub total_billed_duration: Option<String>,
    #[serde(flatten)]
    pub native_fields: Object,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecognizeResponse {
    pub results: Vec<RecognitionResult>,
    pub metadata: Option<ResponseMetadata>,
    #[serde(flatten)]
    pub native_fields: Object,
}
