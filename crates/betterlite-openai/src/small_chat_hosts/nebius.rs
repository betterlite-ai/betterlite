use betterlite_prompts::arguments::json::Object;

use crate::openai_base_config::CatalogCapabilities;

// NebiusConfig keeps the OpenAI list and replaces the parent's mapping
#[derive(Debug, Clone, Copy, Default)]
pub struct NebiusConfig;

impl NebiusConfig {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions - the config API fixes this name and the parent list is the whole body
    pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
        super::openai_parent().supported_params(model, catalog)
    }

    // the parent mapping is not called: max_completion_tokens becomes max_tokens and the rest is copied if listed
    pub fn map_openai_params(
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        super::map_supported(
            non_default_params,
            optional_params,
            &Self::supported_params(model, catalog),
            super::CompletionTokenRename::Rename,
        );
    }
}
