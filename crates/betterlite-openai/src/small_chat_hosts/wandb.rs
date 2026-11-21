use betterlite_prompts::arguments::json::Object;

use crate::edenai_chat::ReasoningLookup;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

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
        let mut params = OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: Headers::new(),
            settings: OpenAiSettings::default(),
        }
        .supported_params(model, catalog);
        if lookup.supports_reasoning(model).unwrap_or(false) {
            params.push(REASONING_EFFORT);
        }
        params
    }

    // the parent mapping is not called, max_completion_tokens becomes max_tokens, and the walk keeps the input order
    pub fn map_openai_params<Lookup>(
        lookup: &Lookup,
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) where
        Lookup: ReasoningLookup,
    {
        super::map_supported(
            non_default_params,
            optional_params,
            &Self::supported_params(lookup, model, catalog),
            super::CompletionTokenRename::Rename,
        );
    }
}
