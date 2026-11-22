use std::collections::BTreeMap;

use crate::cloudflare_decisions::CloudflareDecisionsConfig;
use crate::decisions::openai::config::{Credential, OpenAiDecisionsConfig};
use crate::decisions::transformation::provider_config::{DecisionsProviderConfig, first_secret};
use crate::hosted_vllm_media::decisions::HostedVllmDecisionsConfig;
use crate::openrouter_decisions::OpenRouterDecisionsConfig;
use crate::perplexity_decisions::PerplexityDecisionsConfig;
use crate::providers::strands_decider::StrandsDeciderDecisionsConfig;
use crate::providers::typesafe::decisions::TypeSafeDecisionsConfig;

use crate::decisions::call::azure::AzureAiConfig;
use crate::decisions::call::present;

use super::ProviderConfig;

// The trait's resolve_* read the environment through a bare fn pointer, so the provider's env names are
// resolved here first and handed down as the explicit value, keeping LiteLLM's explicit-then-env order.
impl ProviderConfig {
    #[must_use]
    pub const fn api_key_required(&self) -> bool {
        match self {
            Self::Openai(_config) => {
                <OpenAiDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
            Self::Openrouter(_config) => {
                <OpenRouterDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
            Self::AzureAi(_config) => <AzureAiConfig as DecisionsProviderConfig>::API_KEY_REQUIRED,
            Self::Perplexity(_config) => {
                <PerplexityDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
            Self::Typesafe(_config) => {
                <TypeSafeDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
            Self::StrandsDecider(_config) => {
                <StrandsDeciderDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
            Self::Cloudflare(_config) => {
                <CloudflareDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
            Self::HostedVllm(_config) => {
                <HostedVllmDecisionsConfig as DecisionsProviderConfig>::API_KEY_REQUIRED
            }
        }
    }

    #[must_use]
    pub const fn supports_safety_identifier(&self) -> bool {
        match self {
            Self::Openai(_config) => {
                <OpenAiDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::Openrouter(_config) => {
                <OpenRouterDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::AzureAi(_config) => {
                <AzureAiConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::Perplexity(_config) => {
                <PerplexityDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::Typesafe(_config) => {
                <TypeSafeDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::StrandsDecider(_config) => {
                <StrandsDeciderDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::Cloudflare(_config) => {
                <CloudflareDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
            Self::HostedVllm(_config) => {
                <HostedVllmDecisionsConfig as DecisionsProviderConfig>::SUPPORTS_SAFETY_IDENTIFIER
            }
        }
    }

    #[must_use]
    pub fn resolve_api_base(
        &self,
        api_base: Option<&str>,
        environment: &BTreeMap<String, String>,
    ) -> Option<String> {
        match self {
            Self::Openai(config) => {
                let getter = |name: &str| environment.get(name).cloned().unwrap_or_default();
                config.resolve(present(api_base), Credential::Base, &getter)
            }
            Self::Openrouter(config) => {
                let base = combined(
                    api_base,
                    <OpenRouterDecisionsConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
            Self::AzureAi(config) => {
                let base = combined(
                    api_base,
                    <AzureAiConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
            Self::Perplexity(config) => {
                let base = combined(
                    api_base,
                    <PerplexityDecisionsConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
            Self::Typesafe(config) => {
                let base = combined(
                    api_base,
                    <TypeSafeDecisionsConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
            Self::StrandsDecider(config) => {
                let base = combined(
                    api_base,
                    <StrandsDeciderDecisionsConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
            Self::Cloudflare(config) => {
                let base = combined(
                    api_base,
                    <CloudflareDecisionsConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
            Self::HostedVllm(config) => {
                let base = combined(
                    api_base,
                    <HostedVllmDecisionsConfig as DecisionsProviderConfig>::API_BASE_ENV,
                    environment,
                );
                config.resolve_api_base(base.as_deref(), no_secret)
            }
        }
    }

    #[must_use]
    pub fn resolve_api_key(
        &self,
        api_key: Option<&str>,
        environment: &BTreeMap<String, String>,
    ) -> Option<String> {
        match self {
            Self::Openai(config) => {
                let getter = |name: &str| environment.get(name).cloned().unwrap_or_default();
                config.resolve(present(api_key), Credential::Key, &getter)
            }
            Self::Openrouter(config) => {
                let key = combined(
                    api_key,
                    <OpenRouterDecisionsConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
            Self::AzureAi(config) => {
                let key = combined(
                    api_key,
                    <AzureAiConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
            Self::Perplexity(config) => {
                let key = combined(
                    api_key,
                    <PerplexityDecisionsConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
            Self::Typesafe(config) => {
                let key = combined(
                    api_key,
                    <TypeSafeDecisionsConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
            Self::StrandsDecider(config) => {
                let key = combined(
                    api_key,
                    <StrandsDeciderDecisionsConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
            Self::Cloudflare(config) => {
                let key = combined(
                    api_key,
                    <CloudflareDecisionsConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
            Self::HostedVllm(config) => {
                let key = combined(
                    api_key,
                    <HostedVllmDecisionsConfig as DecisionsProviderConfig>::API_KEY_ENV,
                    environment,
                );
                config.resolve_api_key(key.as_deref(), no_secret)
            }
        }
    }
}

const fn no_secret(_name: &str) -> String {
    String::new()
}

fn environment_secret(names: &[&str], environment: &BTreeMap<String, String>) -> Option<String> {
    first_secret(names, &|name| {
        environment.get(name).cloned().unwrap_or_default()
    })
}

fn combined(
    explicit: Option<&str>,
    names: &[&str],
    environment: &BTreeMap<String, String>,
) -> Option<String> {
    present(explicit)
        .map(str::to_owned)
        .or_else(|| environment_secret(names, environment))
}
