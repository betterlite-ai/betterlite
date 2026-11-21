use betterlite_prompts::arguments::json::Object;

use crate::openai_base_config::CatalogCapabilities;

// NebiusConfig keeps the OpenAI list and replaces the parent's mapping
#[derive(Debug, Clone, Copy, Default)]
pub struct NebiusConfig;

impl NebiusConfig {
    #[must_use]
    pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
        super::parent_supported_params(model, catalog)
    }

    // the parent mapping is not called: max_completion_tokens becomes max_tokens and
    // every other param is copied only when the list carries it
    pub fn map_openai_params(
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let supported = Self::supported_params(model, catalog);
        super::map_with_completion_tokens(non_default_params, optional_params, &supported);
    }
}
