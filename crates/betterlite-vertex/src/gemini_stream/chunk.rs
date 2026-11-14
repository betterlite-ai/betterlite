use crate::gemini_candidates::choice::StreamChoice;
use crate::gemini_metadata::extension::MetadataExtension;
use betterlite_models::usage_sum::Usage;
#[derive(Debug)]
pub struct StreamChunk {
    pub id: String,
    pub created: i64,
    pub model: Option<String>,
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
    pub service_tier: Option<String>,
    pub metadata: MetadataExtension,
    // LiteLLM keeps these two in _hidden_params, and trafficType rides only the chunks that carry usage
    pub traffic_type: Option<String>,
    pub is_finished: bool,
}
#[derive(Debug)]
pub enum Choice {
    Projected(StreamChoice),
    // every candidate reaching this variant was skipped for lacking finishReason, so its mapped reason is none
    EmptyDelta { index: u32 },
}
