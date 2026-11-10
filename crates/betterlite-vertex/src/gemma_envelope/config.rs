use betterlite_openai::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

use crate::model_policy::UNSUPPORTED_DEPLOYED_PARAMS;

pub struct VertexGemmaConfig {
    pub base: OpenAiConfig,
}

impl VertexGemmaConfig {
    // Gemma deployments serve only :predict, so a client stream is synthesized from one complete reply
    pub const SHOULD_FAKE_STREAM: bool = true;

    #[must_use]
    pub fn supported_params(
        &self,
        model: &str,
        catalog: &CatalogCapabilities,
    ) -> Vec<&'static str> {
        self.base
            .supported_params(model, catalog)
            .into_iter()
            .filter(|param| !UNSUPPORTED_DEPLOYED_PARAMS.contains(param))
            .collect()
    }

    pub fn map_openai_params(
        &self,
        model: &str,
        non_default_params: &Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let supported = self.supported_params(model, catalog);
        for (name, value) in non_default_params {
            if supported.contains(&name.as_str()) {
                optional_params.insert(name.clone(), value.clone());
            }
        }
    }
}
