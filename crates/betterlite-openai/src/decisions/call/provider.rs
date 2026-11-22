use std::collections::BTreeMap;
use std::fmt;

use betterlite_models::decisions::ir;

use crate::cloudflare_decisions::CloudflareDecisionsConfig;
use crate::credentials::LiteLlmGlobals;
use crate::decisions::openai::config::{Credential, OpenAiDecisionsConfig};
use crate::decisions::providers::DecisionsProvider;
use crate::decisions::transformation::provider_config::DecisionsProviderConfig;
use crate::decisions::transformation::requests::RequestConversionError;
use crate::decisions::transformation::wire::ProviderRequestWire;
use crate::hosted_vllm_media::decisions::HostedVllmDecisionsConfig;
use crate::openrouter_decisions::OpenRouterDecisionsConfig;
use crate::perplexity_decisions::PerplexityDecisionsConfig;
use crate::providers::strands_decider::StrandsDeciderDecisionsConfig;
use crate::providers::typesafe::decisions::TypeSafeDecisionsConfig;

use self::credentials::combined;
use super::azure::AzureAiConfig;
use super::present;

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

// Every question a selected provider answers. The config trait's associated constants keep it out of a trait
// object, so this blanket impl reads them and a provider's operations exist once rather than once per variant.
pub(crate) trait ProviderOps: Send + Sync {
    fn api_key_required(&self) -> bool;

    fn supports_safety_identifier(&self) -> bool;

    fn api_base_env(&self) -> &'static [&'static str];

    fn api_key_env(&self) -> &'static [&'static str];

    fn canonical_model(&self, model: &str) -> String;

    fn missing_api_base_message(&self, custom_llm_provider: &str) -> String;

    fn get_complete_url(&self, api_base: &str, model: &str) -> String;

    fn transform_decisions_request(
        &self,
        model: &str,
        request: &ir::DecisionRequest,
        custom_llm_provider: &str,
    ) -> Result<ProviderRequestWire, RequestConversionError>;

    fn resolve(&self, credential: Credential, value: Option<&str>) -> Option<String>;
}

impl<T: DecisionsProviderConfig + Send + Sync> ProviderOps for T {
    fn api_key_required(&self) -> bool {
        <T as DecisionsProviderConfig>::API_KEY_REQUIRED
    }

    fn supports_safety_identifier(&self) -> bool {
        <T as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
    }

    fn api_base_env(&self) -> &'static [&'static str] {
        <T as DecisionsProviderConfig>::API_BASE_ENV
    }

    fn api_key_env(&self) -> &'static [&'static str] {
        <T as DecisionsProviderConfig>::API_KEY_ENV
    }

    fn canonical_model(&self, model: &str) -> String {
        DecisionsProviderConfig::canonical_model(self, model)
    }

    fn missing_api_base_message(&self, custom_llm_provider: &str) -> String {
        DecisionsProviderConfig::missing_api_base_message(self, custom_llm_provider)
    }

    fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        DecisionsProviderConfig::get_complete_url(self, api_base, model)
    }

    fn transform_decisions_request(
        &self,
        model: &str,
        request: &ir::DecisionRequest,
        custom_llm_provider: &str,
    ) -> Result<ProviderRequestWire, RequestConversionError> {
        DecisionsProviderConfig::transform_decisions_request(
            self,
            model,
            request,
            custom_llm_provider,
        )
    }

    fn resolve(&self, credential: Credential, value: Option<&str>) -> Option<String> {
        let no_secret = |_name: &str| String::new();
        match credential {
            Credential::Base => DecisionsProviderConfig::resolve_api_base(self, value, no_secret),
            Credential::Key => DecisionsProviderConfig::resolve_api_key(self, value, no_secret),
        }
    }
}

impl ProviderConfig {
    #[must_use]
    pub(crate) fn ops(&self) -> &dyn ProviderOps {
        match self {
            Self::Openai(config) => config.as_ref(),
            Self::Openrouter(config) => config.as_ref(),
            Self::AzureAi(config) => config.as_ref(),
            Self::Perplexity(config) => config.as_ref(),
            Self::Typesafe(config) => config.as_ref(),
            Self::StrandsDecider(config) => config.as_ref(),
            Self::Cloudflare(config) => config.as_ref(),
            Self::HostedVllm(config) => config.as_ref(),
        }
    }

    // OpenAI alone reads globals and environment inside its own resolve, so it gets the environment getter
    // while the other providers take the value resolved from their env names.
    #[must_use]
    pub(crate) fn resolve(
        &self,
        credential: Credential,
        value: Option<&str>,
        environment: &BTreeMap<String, String>,
    ) -> Option<String> {
        if let Self::Openai(openai) = self {
            let getter = |name: &str| environment.get(name).cloned().unwrap_or_default();
            return openai.resolve(present(value), credential, &getter);
        }
        let names = match credential {
            Credential::Base => self.ops().api_base_env(),
            Credential::Key => self.ops().api_key_env(),
        };
        let given = combined(value, names, environment);
        self.ops().resolve(credential, given.as_deref())
    }

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
