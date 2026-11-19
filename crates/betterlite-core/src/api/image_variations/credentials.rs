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
    let supplied_key = supplied_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned();
    let supplied_base = supplied_base
        .filter(|base| !base.is_empty())
        .map(str::to_owned);
    match provider {
        ProviderId::Openai => Credentials {
            api_key: supplied_key.or_else(|| {
                [
                    settings.api_key.clone(),
                    settings.openai_key.clone(),
                    settings.openai_api_key.clone(),
                ]
                .into_iter()
                .flatten()
                .find(|key| !key.expose_secret().is_empty())
            }),
            api_base: supplied_base.or_else(|| {
                let configured = [
                    settings.api_base.as_deref(),
                    settings.openai_base_url.as_deref(),
                    settings.openai_api_base.as_deref(),
                ]
                .into_iter()
                .flatten()
                .find(|value| !value.is_empty());
                Some(configured.unwrap_or(DEFAULT_OPENAI_API_BASE).to_owned())
            }),
        },
        ProviderId::Topaz => Credentials {
            api_key: supplied_key.or_else(|| {
                settings
                    .topaz_api_key
                    .as_ref()
                    .filter(|key| !key.expose_secret().is_empty())
                    .cloned()
            }),
            api_base: supplied_base.or_else(|| {
                let configured = settings
                    .topaz_api_base
                    .as_deref()
                    .filter(|value| !value.is_empty());
                Some(configured.unwrap_or(DEFAULT_TOPAZ_API_BASE).to_owned())
            }),
        },
        _ => Credentials {
            api_key: None,
            api_base: None,
        },
    }
}
