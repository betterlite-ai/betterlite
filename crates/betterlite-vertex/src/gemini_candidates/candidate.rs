use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::gemini_response_parts::projection::{LogprobsResult, Part};

// the metadata lists stay in `metadata` as sent, because B1323 reads them from the raw candidate
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub content: Option<Content>,
    pub finish_reason: Option<String>,
    pub index: Option<u32>,
    pub logprobs_result: Option<LogprobsResult>,
    #[serde(flatten)]
    pub metadata: IndexMap<String, Json>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Content {
    pub parts: Option<Vec<Part>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedback {
    pub block_reason: Option<String>,
}
