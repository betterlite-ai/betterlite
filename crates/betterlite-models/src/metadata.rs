use identity::{AudioFormat, Modality, Mode};
use levels::ReasoningEffort;
use notes::CatalogNotes;
use serde::{Deserialize, Serialize};
use vendor::{VertexAudioApi, WebSearchBillingUnit};

pub mod identity;
pub mod levels;
pub mod notes;
pub mod vendor;

/// The non-price, non-capability fields of a pinned catalog entry.
///
/// Covers provider and mode identity, token/context limits, endpoint/modality/region support,
/// reasoning levels, and deprecation/source documentation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataFields {
    pub litellm_provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpm: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_min_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_vector_size: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supported_endpoints: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supported_regions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supported_modalities: Vec<Modality>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supported_output_modalities: Vec<Modality>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub supported_audio_formats: Vec<AudioFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_transcription_config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasoning_effort_levels: Vec<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bedrock_output_config_effort_ceiling: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertex_ai_audio_api: Option<VertexAudioApi>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_billing_unit: Option<WebSearchBillingUnit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecation_date: Option<chrono::NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "metadata")]
    pub notes: CatalogNotes,
}

impl MetadataFields {
    // Lets entry::unknown_paths recognize every field here, and recovers a
    // path lost through the #[serde(flatten)] that composes ModelFields.
    pub(crate) const WIRE_NAMES: &'static [&'static str] = &[
        "litellm_provider",
        "mode",
        "max_input_tokens",
        "max_output_tokens",
        "max_tokens",
        "rpm",
        "tpm",
        "prompt_cache_min_tokens",
        "output_vector_size",
        "supported_endpoints",
        "supported_regions",
        "supported_modalities",
        "supported_output_modalities",
        "supported_audio_formats",
        "audio_transcription_config",
        "default_reasoning_effort",
        "reasoning_effort_levels",
        "bedrock_output_config_effort_ceiling",
        "vertex_ai_audio_api",
        "web_search_billing_unit",
        "deprecation_date",
        "source",
        "comment",
        "metadata",
    ];
}

mod tests;
