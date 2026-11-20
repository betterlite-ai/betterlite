use std::collections::BTreeMap;

use betterlite_models::call_helpers::drop_params::drop_params_env_flag;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup, parse_view};
use crate::resolve::{resolve_secret, resolve_string};

mod tests;

pub type Headers = BTreeMap<String, String>;

// The rerank dispatcher reads litellm_settings through this view; every other section keeps its own view.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    api_key: Option<SecretString>,
    api_base: Option<String>,
    #[serde(default)]
    headers: Headers,
    togetherai_api_key: Option<SecretString>,
    drop_params: Option<bool>,
}

// The provider variables the dispatcher's last-resort fallbacks read; an empty one counts as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    pub cohere_api_base: Option<String>,
    pub azure_ai_api_base: Option<String>,
    pub infinity_api_base: Option<String>,
    pub bedrock_api_base: Option<String>,
    pub togetherai_api_key: Option<SecretString>,
    pub jina_ai_api_key: Option<SecretString>,
    pub nvidia_nim_api_key: Option<SecretString>,
    pub nvidia_nim_api_base: Option<String>,
    pub hosted_vllm_api_key: Option<SecretString>,
    pub hosted_vllm_api_base: Option<String>,
    pub deepinfra_api_key: Option<SecretString>,
    pub deepinfra_api_base: Option<String>,
    pub fireworks_api_key: Option<SecretString>,
    pub fireworks_ai_api_key: Option<SecretString>,
    pub fireworksai_api_key: Option<SecretString>,
    pub fireworks_ai_token: Option<SecretString>,
    pub fireworks_ai_api_base: Option<String>,
    pub voyage_api_key: Option<SecretString>,
    pub voyage_ai_api_key: Option<SecretString>,
    pub voyage_api_base: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RerankSettings {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub headers: Option<Headers>,
    pub togetherai_api_key: Option<SecretString>,
    pub drop_params: bool,
    pub environment: Environment,
}

// The settings litellm_settings holds for rerank, read through the same YAML decoding the config loader uses.
pub fn loads_rerank_settings(
    source: &str,
    environment: &dyn EnvironmentLookup,
) -> Result<RerankSettings, ConfigError> {
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    raw.convert(environment)
}

impl Raw {
    fn convert(self, lookup: &dyn EnvironmentLookup) -> Result<RerankSettings, ConfigError> {
        let settings = self.litellm_settings;
        let mut headers = Headers::new();
        for (name, value) in settings.headers {
            let resolved = resolve_string(value, "litellm_settings.headers", lookup)?;
            headers.insert(name, resolved);
        }
        let environment_flag =
            drop_params_env_flag(lookup.lookup("LITELLM_DROP_PARAMS").as_deref());
        if environment_flag.warned {
            tracing::warn!("LITELLM_DROP_PARAMS is not a flag value, treating it as on");
        }
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Ok(RerankSettings {
            api_base: match settings.api_base {
                Some(text) => Some(resolve_string(text, "litellm_settings.api_base", lookup)?),
                None => None,
            }
            .filter(|value| !value.is_empty()),
            api_key: resolve_secret(settings.api_key, "litellm_settings.api_key", lookup)?
                .filter(|value| !value.expose_secret().is_empty()),
            headers: (!headers.is_empty()).then_some(headers),
            togetherai_api_key: resolve_secret(
                settings.togetherai_api_key,
                "litellm_settings.togetherai_api_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            drop_params: settings
                .drop_params
                .or(Some(environment_flag.enabled))
                .unwrap_or(false),
            environment: Environment {
                cohere_api_base: read("COHERE_API_BASE"),
                azure_ai_api_base: read("AZURE_AI_API_BASE"),
                infinity_api_base: read("INFINITY_API_BASE"),
                bedrock_api_base: read("BEDROCK_API_BASE"),
                togetherai_api_key: read("TOGETHERAI_API_KEY").map(SecretString::from),
                jina_ai_api_key: read("JINA_AI_API_KEY").map(SecretString::from),
                nvidia_nim_api_key: read("NVIDIA_NIM_API_KEY").map(SecretString::from),
                nvidia_nim_api_base: read("NVIDIA_NIM_API_BASE"),
                hosted_vllm_api_key: read("HOSTED_VLLM_API_KEY").map(SecretString::from),
                hosted_vllm_api_base: read("HOSTED_VLLM_API_BASE"),
                deepinfra_api_key: read("DEEPINFRA_API_KEY").map(SecretString::from),
                deepinfra_api_base: read("DEEPINFRA_API_BASE"),
                fireworks_api_key: read("FIREWORKS_API_KEY").map(SecretString::from),
                fireworks_ai_api_key: read("FIREWORKS_AI_API_KEY").map(SecretString::from),
                fireworksai_api_key: read("FIREWORKSAI_API_KEY").map(SecretString::from),
                fireworks_ai_token: read("FIREWORKS_AI_TOKEN").map(SecretString::from),
                fireworks_ai_api_base: read("FIREWORKS_AI_API_BASE"),
                voyage_api_key: read("VOYAGE_API_KEY").map(SecretString::from),
                voyage_ai_api_key: read("VOYAGE_AI_API_KEY").map(SecretString::from),
                voyage_api_base: read("VOYAGE_API_BASE"),
            },
        })
    }
}
