use betterlite_config::sections::image_variations::Settings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_metadata_factory::selection::{ProviderModelMetadata, select};
use secrecy::{ExposeSecret as _, SecretString};

pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";
pub const DEFAULT_TOPAZ_API_BASE: &str = "https://api.topazlabs.com";

// OpenAIGPTConfig and TopazModelInfo expose get_api_key and get_api_base over the same settings
#[derive(Debug, Clone)]
pub struct Credentials {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

// ProviderConfigManager.get_provider_model_info and the two config objects' credential chains
pub struct VariationPolicies {
    pub model_info: fn(Option<&str>, ProviderId) -> Option<ProviderModelMetadata>,
    pub credentials: fn(ProviderId, Option<&SecretString>, Option<&str>, &Settings) -> Credentials,
}

impl Default for VariationPolicies {
    fn default() -> Self {
        Self {
            model_info: select,
            credentials: provider_credentials,
        }
    }
}

// a supplied key or base wins while it is not empty, then the provider's own chain answers
pub(crate) fn provider_credentials(
    provider: ProviderId,
    supplied_key: Option<&SecretString>,
    supplied_base: Option<&str>,
    settings: &Settings,
) -> Credentials {
    let key = supplied_key
        .filter(|given| !given.expose_secret().is_empty())
        .cloned();
    let base = supplied_base
        .filter(|given| !given.is_empty())
        .map(str::to_owned);
    if provider == ProviderId::Openai {
        return openai_credentials(key, base, settings);
    }
    if provider == ProviderId::Topaz {
        return topaz_credentials(key, base, settings);
    }
    Credentials {
        api_key: None,
        api_base: None,
    }
}

// OpenAIGPTConfig's chain: api_key, openai_key, openai_api_key, then OPENAI_API_BASE's default
fn openai_credentials(
    key: Option<SecretString>,
    base: Option<String>,
    settings: &Settings,
) -> Credentials {
    let configured_key = [
        settings.api_key.clone(),
        settings.openai_key.clone(),
        settings.openai_api_key.clone(),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| !candidate.expose_secret().is_empty());
    let configured_base = [
        settings.api_base.as_deref(),
        settings.openai_base_url.as_deref(),
        settings.openai_api_base.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| !candidate.is_empty());
    let resolved_base = base.unwrap_or_else(|| {
        configured_base
            .unwrap_or(DEFAULT_OPENAI_API_BASE)
            .to_owned()
    });
    Credentials {
        api_key: key.or(configured_key),
        api_base: Some(resolved_base),
    }
}

// TopazModelInfo's chain: topaz_api_key and topaz_api_base, then the Topaz default base
fn topaz_credentials(
    key: Option<SecretString>,
    base: Option<String>,
    settings: &Settings,
) -> Credentials {
    let configured_key = settings
        .topaz_api_key
        .as_ref()
        .filter(|candidate| !candidate.expose_secret().is_empty())
        .cloned();
    let configured_base = settings
        .topaz_api_base
        .as_deref()
        .filter(|candidate| !candidate.is_empty());
    let resolved_base =
        base.unwrap_or_else(|| configured_base.unwrap_or(DEFAULT_TOPAZ_API_BASE).to_owned());
    Credentials {
        api_key: key.or(configured_key),
        api_base: Some(resolved_base),
    }
}
