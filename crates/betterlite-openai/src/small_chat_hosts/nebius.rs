use betterlite_prompts::arguments::json::Object;

use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

// NebiusConfig keeps the OpenAI list and replaces the parent's mapping
#[derive(Debug, Clone, Copy, Default)]
pub struct NebiusConfig;

impl NebiusConfig {
    // the parent mapping is not called: max_completion_tokens becomes max_tokens and the rest is copied if listed
    pub fn map_openai_params(
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let supported = OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        }
        .supported_params(model, catalog);
        super::map_supported(
            non_default_params,
            optional_params,
            &supported,
            super::CompletionTokenRename::Rename,
        );
    }
}
