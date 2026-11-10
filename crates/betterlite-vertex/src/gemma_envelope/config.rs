use std::collections::BTreeMap;

use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, OpenAiConfig, OpenAiSettings,
};

use crate::model_policy::UNSUPPORTED_DEPLOYED_PARAMS;

pub struct VertexGemmaConfig;

impl VertexGemmaConfig {
    // Gemma deployments serve only :predict, so a client stream is synthesized from one complete reply
    pub const SHOULD_FAKE_STREAM: bool = true;

    #[must_use]
    pub fn supported_params(
        &self,
        model: &str,
        catalog: &CatalogCapabilities,
    ) -> Vec<&'static str> {
        // LiteLLM's VertexGemmaConfig extends the plain OpenAIGPTConfig, which carries no OpenAI-provider extras
        OpenAiConfig {
            family: ConfigFamily::Other,
            api_key: None,
            headers: BTreeMap::new(),
            settings: OpenAiSettings::default(),
        }
        .supported_params(model, catalog)
        .into_iter()
        .filter(|param| !UNSUPPORTED_DEPLOYED_PARAMS.contains(param))
        .collect()
    }
}
