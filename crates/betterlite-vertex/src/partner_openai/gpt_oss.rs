use betterlite_openai::openai_base_config::{CatalogCapabilities, ConfigFamily, Object};

use super::params::{base_config, map_supported};

// LiteLLM's removal list spells its first entry "tool", so the inherited "tools" stays supported.
const TOOL_CALLING_PARAMS: [&str; 4] = ["tool", "tool_choice", "function_call", "functions"];

// The transformation owns no class attributes, so its configuration snapshot is empty.
pub struct GptOssConfig;

impl GptOssConfig {
    #[must_use]
    pub fn supported_params(
        model: &str,
        catalog: &CatalogCapabilities,
        supports_function_calling: Option<bool>,
    ) -> Vec<&'static str> {
        let mut params = base_config(ConfigFamily::Other).supported_params(model, catalog);
        params.push("reasoning_effort");
        if supports_function_calling == Some(false) {
            params.retain(|param| !TOOL_CALLING_PARAMS.contains(param));
        }
        params
    }

    pub fn map_params(
        model: &str,
        requested: &Object,
        existing: &mut Object,
        catalog: &CatalogCapabilities,
        supports_function_calling: Option<bool>,
    ) {
        let supported = Self::supported_params(model, catalog, supports_function_calling);
        map_supported(&supported, requested, existing);
    }
}
