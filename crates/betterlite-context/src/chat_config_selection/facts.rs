use crate::bedrock_config_selection::chat::{BedrockInvokeProvider, BedrockRoute};

pub trait ModelFamilyProbe {
    #[must_use]
    fn openai_o_series(&self, model: &str) -> bool;
    #[must_use]
    fn openai_gpt_5(&self, model: &str) -> bool;
    #[must_use]
    fn openai_catalog_model(&self, model: &str) -> bool;
    #[must_use]
    fn azure_gpt_5(&self, model: &str) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub struct BedrockFacts<'source> {
    pub route: BedrockRoute,
    pub invoke_provider: Option<BedrockInvokeProvider>,
    pub base_model: &'source str,
}
