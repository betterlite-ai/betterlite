use betterlite_models::decisions::{HiddenParams, Json};
use indexmap::IndexMap;

use super::provider::GenAiProvider;

// The attributes LiteLLM's base iterator keeps; B3488 fills the hidden params before the first event
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenAiStreamMetadata {
    pub model: String,
    pub provider: GenAiProvider,
    pub request_body: IndexMap<String, Json>,
    pub hidden_params: HiddenParams,
}
