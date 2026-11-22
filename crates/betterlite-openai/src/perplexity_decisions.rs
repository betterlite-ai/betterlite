mod tests;

use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

pub const DEFAULT_API_BASE: &str = "https://api.perplexity.ai";

#[derive(Debug, Clone, Copy)]
pub struct PerplexityDecisionsConfig;

impl DecisionsProviderConfig for PerplexityDecisionsConfig {
    const PATH: &'static str = "/v1/decisions";
    const API_KEY_ENV: &'static [&'static str] = &["PERPLEXITYAI_API_KEY", "PERPLEXITY_API_KEY"];
    const API_BASE_ENV: &'static [&'static str] = &["PERPLEXITY_API_BASE"];

    fn get_default_api_base(&self) -> Option<String> {
        Some(String::from(DEFAULT_API_BASE))
    }
}
