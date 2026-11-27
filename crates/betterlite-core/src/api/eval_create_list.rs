pub mod call;
pub mod config;
pub mod error;
pub mod handler;
pub mod logging;
pub mod request;
pub mod service;

mod tests;

// LiteLLM's DEFAULT_OPENAI_API_BASE, the base create_eval uses when litellm_params carries none
pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com";

// LiteLLM's custom_llm_provider default, applied before ProviderConfigManager sees the name
pub const DEFAULT_EVALS_PROVIDER: &str = "openai";
