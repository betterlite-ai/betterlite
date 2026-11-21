use betterlite_prompts::arguments::json::Object;

use crate::edenai_chat::ReasoningLookup;
use crate::openai_base_config::CatalogCapabilities;

const REASONING_EFFORT: &str = "reasoning_effort";

// WandbConfig keeps the OpenAI list plus reasoning_effort, and replaces the parent's mapping
#[derive(Debug, Clone, Copy, Default)]
pub struct WandbConfig;

impl WandbConfig {
    #[must_use]
    pub fn supported_params<Lookup>(
        lookup: &Lookup,
        model: &str,
        catalog: &CatalogCapabilities,
    ) -> Vec<&'static str>
    where
        Lookup: ReasoningLookup,
    {
        let mut params = super::parent_supported_params(model, catalog);
        if super::supports_reasoning(lookup, model) {
            params.push(REASONING_EFFORT);
        }
        params
    }

    // the parent mapping is not called, and the walk keeps the input order
    pub fn map_openai_params<Lookup>(
        lookup: &Lookup,
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) where
        Lookup: ReasoningLookup,
    {
        let supported = Self::supported_params(lookup, model, catalog);
        super::map_with_completion_tokens(non_default_params, optional_params, &supported);
    }
}
