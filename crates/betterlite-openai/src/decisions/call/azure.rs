use std::fmt;

use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

pub const MISSING_API_BASE_MESSAGE: &str = "Missing AZURE_AI_API_BASE - set AZURE_AI_API_BASE or pass api_base, e.g. https://<resource>.services.ai.azure.com";

// betterlite-foundry owns the real Azure AI config and depends on this crate, so the route builder arrives as a closure
pub struct AzureAiConfig {
    pub complete_url: Box<dyn Fn(&str, &str) -> String + Send + Sync>,
}

impl fmt::Debug for AzureAiConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureAiConfig")
            .finish_non_exhaustive()
    }
}

impl DecisionsProviderConfig for AzureAiConfig {
    const PATH: &'static str = "/providers/microsoft/v1/systemone";
    const API_KEY_ENV: &'static [&'static str] = &["AZURE_AI_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["AZURE_AI_API_BASE"];

    fn missing_api_base_message(&self, _custom_llm_provider: &str) -> String {
        MISSING_API_BASE_MESSAGE.to_owned()
    }

    fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        (self.complete_url)(api_base, model)
    }
}
