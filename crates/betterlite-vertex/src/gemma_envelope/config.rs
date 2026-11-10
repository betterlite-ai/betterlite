use betterlite_openai::openai_base_config::{CatalogCapabilities, OpenAiConfig};

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
}
