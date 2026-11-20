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

// one provider's configured key and base sources, in LiteLLM's order, with its default base
struct CredentialChain<'settings> {
    keys: [Option<&'settings SecretString>; 3],
    bases: [Option<&'settings str>; 3],
    default_base: &'static str,
}

impl CredentialChain<'_> {
    fn resolve(self, key: Option<SecretString>, base: Option<String>) -> Credentials {
        let configured_key = self
            .keys
            .into_iter()
            .flatten()
            .find(|candidate| !candidate.expose_secret().is_empty())
            .cloned();
        let configured_base = self
            .bases
            .into_iter()
            .flatten()
            .find(|candidate| !candidate.is_empty());
        let resolved_base =
            base.unwrap_or_else(|| configured_base.unwrap_or(self.default_base).to_owned());
        Credentials {
            api_key: key.or(configured_key),
            api_base: Some(resolved_base),
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
    let chain = if provider == ProviderId::Openai {
        // OpenAIGPTConfig's chain: api_key, openai_key, openai_api_key, then OPENAI_API_BASE's default
        CredentialChain {
            keys: [
                settings.api_key.as_ref(),
                settings.openai_key.as_ref(),
                settings.openai_api_key.as_ref(),
            ],
            bases: [
                settings.api_base.as_deref(),
                settings.openai_base_url.as_deref(),
                settings.openai_api_base.as_deref(),
            ],
            default_base: DEFAULT_OPENAI_API_BASE,
        }
    } else if provider == ProviderId::Topaz {
        // TopazModelInfo's chain: topaz_api_key and topaz_api_base, then the Topaz default base
        CredentialChain {
            keys: [settings.topaz_api_key.as_ref(), None, None],
            bases: [settings.topaz_api_base.as_deref(), None, None],
            default_base: DEFAULT_TOPAZ_API_BASE,
        }
    } else {
        return Credentials {
            api_key: None,
            api_base: None,
        };
    };
    chain.resolve(key, base)
}
