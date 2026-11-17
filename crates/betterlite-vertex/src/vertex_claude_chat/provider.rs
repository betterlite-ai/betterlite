use betterlite_models::provider_model_sets::{ProviderModelSet, ProviderModelSets};

use super::PROVIDER;

pub const PROVIDER_BETA: &str = "vertex_ai_beta";

// LiteLLM accepts the two Vertex routing names, any model carrying "claude", and every model its
// vertex_anthropic_models catalog lists.
#[must_use]
pub fn is_supported_model(
    model: &str,
    custom_llm_provider: &str,
    sets: &ProviderModelSets,
) -> bool {
    if custom_llm_provider != PROVIDER && custom_llm_provider != PROVIDER_BETA {
        return false;
    }
    model.to_lowercase().contains("claude")
        || sets
            .sets
            .get(&ProviderModelSet::VertexAnthropic)
            .is_some_and(|models| models.contains(model))
}
