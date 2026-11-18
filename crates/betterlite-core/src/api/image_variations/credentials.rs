use betterlite_config::sections::image_variations::Settings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_metadata_factory::selection::{ProviderModelMetadata, select};
use secrecy::{ExposeSecret as _, SecretString};

pub const DEFAULT_OPENAI_API_BASE: &str = "https://api.openai.com/v1";
pub const DEFAULT_TOPAZ_API_BASE: &str = "https://api.topazlabs.com";

// ProviderConfigManager.get_provider_model_info and the two config objects' credential chains.
// The image variation entry reads these after resolution and ignores the resolver's key and base.
pub struct VariationPolicies {
    pub model_info: fn(&str, ProviderId) -> Option<ProviderModelMetadata>,
    pub api_key: fn(ProviderId, Option<&SecretString>, &Settings) -> Option<SecretString>,
    pub api_base: fn(ProviderId, Option<&str>, &Settings) -> Option<String>,
}

impl Default for VariationPolicies {
    fn default() -> Self {
        Self {
            model_info: provider_model_info,
            api_key: provider_api_key,
            api_base: provider_api_base,
        }
    }
}

fn provider_model_info(model: &str, provider: ProviderId) -> Option<ProviderModelMetadata> {
    select(Some(model), provider)
}

// OpenAIGPTConfig.get_api_key and TopazModelInfo.get_api_key, each falling through an empty value
fn provider_api_key(
    provider: ProviderId,
    supplied: Option<&SecretString>,
    settings: &Settings,
) -> Option<SecretString> {
    if let Some(key) = supplied.filter(|key| !key.expose_secret().is_empty()) {
        return Some(key.clone());
    }
    let key = match provider {
        ProviderId::Openai => settings
            .api_key
            .clone()
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| {
                settings
                    .openai_key
                    .clone()
                    .filter(|key| !key.expose_secret().is_empty())
            })
            .or_else(|| {
                settings
                    .openai_api_key
                    .clone()
                    .filter(|key| !key.expose_secret().is_empty())
            }),
        ProviderId::Topaz => settings
            .topaz_api_key
            .clone()
            .filter(|key| !key.expose_secret().is_empty()),
        _ => None,
    };
    key
}

// OpenAIGPTConfig.get_api_base and TopazModelInfo.get_api_base, both with a hardcoded last resort
fn provider_api_base(
    provider: ProviderId,
    supplied: Option<&str>,
    settings: &Settings,
) -> Option<String> {
    if let Some(base) = supplied.filter(|base| !base.is_empty()) {
        return Some(base.to_owned());
    }
    let base = match provider {
        ProviderId::Openai => settings
            .api_base
            .clone()
            .filter(|base| !base.is_empty())
            .or_else(|| {
                settings
                    .openai_base_url
                    .clone()
                    .filter(|base| !base.is_empty())
            })
            .or_else(|| {
                settings
                    .openai_api_base
                    .clone()
                    .filter(|base| !base.is_empty())
            })
            .or_else(|| Some(DEFAULT_OPENAI_API_BASE.to_owned())),
        ProviderId::Topaz => settings
            .topaz_api_base
            .clone()
            .filter(|base| !base.is_empty())
            .or_else(|| Some(DEFAULT_TOPAZ_API_BASE.to_owned())),
        _ => None,
    };
    base
}
