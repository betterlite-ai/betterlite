use serde::{Deserialize, Serialize};

use super::enums::ResponseModality;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Usage {
    pub total_input_tokens: Option<i64>,
    pub input_tokens_by_modality: Option<Vec<ModalityTokens>>,
    pub total_cached_tokens: Option<i64>,
    pub cached_tokens_by_modality: Option<Vec<ModalityTokens>>,
    pub total_output_tokens: Option<i64>,
    pub output_tokens_by_modality: Option<Vec<ModalityTokens>>,
    pub total_tool_use_tokens: Option<i64>,
    pub tool_use_tokens_by_modality: Option<Vec<ModalityTokens>>,
    pub total_reasoning_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ModalityTokens {
    pub modality: Option<ResponseModality>,
    pub tokens: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Annotation {
    pub start_index: Option<i64>,
    pub end_index: Option<i64>,
    pub source: Option<String>,
}
