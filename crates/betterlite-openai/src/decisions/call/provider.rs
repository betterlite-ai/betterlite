use std::collections::BTreeMap;
use std::fmt;

use betterlite_models::decisions::ir;

use crate::cloudflare_decisions::CloudflareDecisionsConfig;
use crate::credentials::LiteLlmGlobals;
use crate::decisions::openai::config::OpenAiDecisionsConfig;
use crate::decisions::providers::DecisionsProvider;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use crate::decisions::transformation::requests::RequestConversionError;
use crate::decisions::transformation::wire::ProviderRequestWire;
use crate::hosted_vllm_media::decisions::HostedVllmDecisionsConfig;
use crate::openrouter_decisions::OpenRouterDecisionsConfig;
use crate::perplexity_decisions::PerplexityDecisionsConfig;
use crate::providers::strands_decider::StrandsDeciderDecisionsConfig;
use crate::providers::typesafe::decisions::TypeSafeDecisionsConfig;

use super::azure::AzureAiConfig;

mod credentials;

// ProviderConfigManager.get_provider_decisions_config returns one concrete config per provider; the enum keeps
// every config's associated constants, which a trait object cannot. Every variant is boxed so the sizes match.
pub enum ProviderConfig {
    Openai(Box<OpenAiDecisionsConfig>),
    Openrouter(Box<OpenRouterDecisionsConfig>),
    AzureAi(Box<AzureAiConfig>),
    Perplexity(Box<PerplexityDecisionsConfig>),
    Typesafe(Box<TypeSafeDecisionsConfig>),
    StrandsDecider(Box<StrandsDeciderDecisionsConfig>),
    Cloudflare(Box<CloudflareDecisionsConfig>),
    HostedVllm(Box<HostedVllmDecisionsConfig>),
}

impl ProviderConfig {
    #[must_use]
    pub fn select(
        provider: DecisionsProvider,
        globals: LiteLlmGlobals,
        environment: &BTreeMap<String, String>,
        azure_ai: AzureAiConfig,
    ) -> Self {
        match provider {
            DecisionsProvider::Openai => Self::Openai(Box::new(OpenAiDecisionsConfig { globals })),
            DecisionsProvider::Openrouter => Self::Openrouter(Box::new(OpenRouterDecisionsConfig)),
            DecisionsProvider::AzureAi => Self::AzureAi(Box::new(azure_ai)),
            DecisionsProvider::Perplexity => Self::Perplexity(Box::new(PerplexityDecisionsConfig)),
            DecisionsProvider::Typesafe => Self::Typesafe(Box::new(TypeSafeDecisionsConfig)),
            DecisionsProvider::StrandsDecider => {
                Self::StrandsDecider(Box::new(StrandsDeciderDecisionsConfig::default()))
            }
            DecisionsProvider::Cloudflare => {
                let account_id = environment.get("CLOUDFLARE_ACCOUNT_ID").cloned();
                Self::Cloudflare(Box::new(CloudflareDecisionsConfig::new(account_id)))
            }
            DecisionsProvider::HostedVllm => Self::HostedVllm(Box::new(HostedVllmDecisionsConfig)),
        }
    }

    #[must_use]
    pub fn canonical_model(&self, model: &str) -> String {
        match self {
            Self::Openai(config) => config.canonical_model(model),
            Self::Openrouter(config) => config.canonical_model(model),
            Self::AzureAi(config) => config.canonical_model(model),
            Self::Perplexity(config) => config.canonical_model(model),
            Self::Typesafe(config) => config.canonical_model(model),
            Self::StrandsDecider(config) => config.canonical_model(model),
            Self::Cloudflare(config) => config.canonical_model(model),
            Self::HostedVllm(config) => config.canonical_model(model),
        }
    }

    #[must_use]
    pub fn missing_api_base_message(&self, provider: &str) -> String {
        match self {
            Self::Openai(config) => config.missing_api_base_message(provider),
            Self::Openrouter(config) => config.missing_api_base_message(provider),
            Self::AzureAi(config) => config.missing_api_base_message(provider),
            Self::Perplexity(config) => config.missing_api_base_message(provider),
            Self::Typesafe(config) => config.missing_api_base_message(provider),
            Self::StrandsDecider(config) => config.missing_api_base_message(provider),
            Self::Cloudflare(config) => config.missing_api_base_message(provider),
            Self::HostedVllm(config) => config.missing_api_base_message(provider),
        }
    }

    #[must_use]
    pub fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        match self {
            Self::Openai(config) => config.get_complete_url(api_base, model),
            Self::Openrouter(config) => config.get_complete_url(api_base, model),
            Self::AzureAi(config) => config.get_complete_url(api_base, model),
            Self::Perplexity(config) => config.get_complete_url(api_base, model),
            Self::Typesafe(config) => config.get_complete_url(api_base, model),
            Self::StrandsDecider(config) => config.get_complete_url(api_base, model),
            Self::Cloudflare(config) => config.get_complete_url(api_base, model),
            Self::HostedVllm(config) => config.get_complete_url(api_base, model),
        }
    }

    pub fn transform_decisions_request(
        &self,
        model: &str,
        request: &ir::DecisionRequest,
        custom_llm_provider: &str,
    ) -> Result<ProviderRequestWire, RequestConversionError> {
        match self {
            Self::Openai(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::Openrouter(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::AzureAi(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::Perplexity(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::Typesafe(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::StrandsDecider(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::Cloudflare(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
            Self::HostedVllm(config) => {
                config.transform_decisions_request(model, request, custom_llm_provider)
            }
        }
    }
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Openai(_config) => "Openai",
            Self::Openrouter(_config) => "Openrouter",
            Self::AzureAi(_config) => "AzureAi",
            Self::Perplexity(_config) => "Perplexity",
            Self::Typesafe(_config) => "Typesafe",
            Self::StrandsDecider(_config) => "StrandsDecider",
            Self::Cloudflare(_config) => "Cloudflare",
            Self::HostedVllm(_config) => "HostedVllm",
        };
        formatter.write_str(name)
    }
}
