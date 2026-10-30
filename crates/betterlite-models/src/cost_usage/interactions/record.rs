use serde::Deserialize;

use crate::cost_usage::field::Field;
use crate::cost_usage::interactions::modality::ModalityList;
use crate::cost_usage::interactions::search::GroundingToolCount;
use crate::cost_usage::token_count::TokenCount;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct InteractionsUsage {
    pub(super) prompt_tokens: Field<TokenCount>,
    pub(super) input_tokens: Field<TokenCount>,
    pub(super) total_input_tokens: Field<TokenCount>,
    pub(super) total_output_tokens: Field<TokenCount>,
    pub(super) total_tool_use_tokens: TokenCount,
    pub(super) total_cached_tokens: TokenCount,
    pub(super) total_reasoning_tokens: TokenCount,
    pub(super) total_thought_tokens: TokenCount,
    pub(super) total_tokens: TokenCount,
    pub(super) input_tokens_by_modality: ModalityList,
    pub(super) tool_use_tokens_by_modality: ModalityList,
    pub(super) cached_tokens_by_modality: ModalityList,
    pub(super) output_tokens_by_modality: ModalityList,
    pub(super) grounding_tool_count: GroundingToolCount,
}

// A present prompt_tokens or input_tokens disqualifies, even when its value is null.
#[must_use]
pub const fn is_interactions_usage_object(usage: &InteractionsUsage) -> bool {
    if usage.prompt_tokens.is_present() || usage.input_tokens.is_present() {
        return false;
    }
    usage.total_input_tokens.is_present() || usage.total_output_tokens.is_present()
}

pub(super) const fn field_value(field: &Field<TokenCount>) -> i64 {
    match field {
        Field::Present(count) => count.0,
        Field::Absent => 0,
    }
}

mod tests;
