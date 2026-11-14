use std::error::Error;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_prompts::arguments::json::{Json, Object};

#[derive(Debug, Clone, Copy)]
pub struct GenerateContentRequest<'request> {
    pub model: &'request str,
    pub contents: &'request Json,
    pub tools: Option<&'request Json>,
    pub generation_config: &'request Object,
    pub system_instruction: Option<&'request Json>,
}

// The seam over B3480's config trait: this crate cannot name a type from betterlite-vertex, which depends on it.
pub trait GenerateContentProviderConfig {
    type Error: Error + Send + Sync + 'static;

    fn map_optional_params(&self, config: &Object, model: &str) -> Object;

    fn transform_request(&self, request: &GenerateContentRequest<'_>) -> Result<Json, Self::Error>;

    // LiteLLM's get_generate_content_request_top_level_fields; the caller's config says which fields sit beside generationConfig
    fn request_top_level_fields(&self) -> &'static [&'static str];
}

pub struct GenerateContentProviderConfigs<'configs, C> {
    pub gemini: &'configs C,
    pub vertex: &'configs C,
    pub is_vertex_partner_model: fn(&str) -> bool,
}

// ProviderConfigManager.get_provider_google_genai_generate_content_config: None means the chat adapter handles the call
#[must_use]
pub fn provider_config_for<'configs, C>(
    model: &str,
    provider: &ProviderIdentity,
    configs: &'configs GenerateContentProviderConfigs<'configs, C>,
) -> Option<&'configs C> {
    match provider {
        ProviderIdentity::Known(ProviderId::Gemini) => Some(configs.gemini),
        ProviderIdentity::Known(ProviderId::VertexAi) => {
            (!(configs.is_vertex_partner_model)(model)).then_some(configs.vertex)
        }
        ProviderIdentity::Known(_) | ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => {
            None
        }
    }
}
