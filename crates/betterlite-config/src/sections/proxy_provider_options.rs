mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    use_litellm_proxy: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    // neither variable has a default in LiteLLM, so an unset one stays None
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub environment_use_litellm_proxy: bool,
    pub global_use_litellm_proxy: bool,
    pub deployment_use_litellm_proxy: Vec<bool>,
}

impl Settings {
    // LiteLLM ORs the three sources, so a false deployment flag never vetoes a true environment or global one
    #[must_use]
    pub fn should_use_litellm_proxy(&self, deployment: usize) -> bool {
        self.environment_use_litellm_proxy
            || self.global_use_litellm_proxy
            || self
                .deployment_use_litellm_proxy
                .get(deployment)
                .copied()
                .unwrap_or(false)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let api_base = lookup
            .lookup("LITELLM_PROXY_API_BASE")
            .filter(|value| !value.is_empty());
        let api_key = lookup
            .lookup("LITELLM_PROXY_API_KEY")
            .filter(|value| !value.is_empty())
            .map(SecretString::from);
        let environment_use_litellm_proxy = lookup
            .lookup("USE_LITELLM_PROXY")
            .map(|text| betterlite_env::flag("USE_LITELLM_PROXY", &text))
            .transpose()?
            .flatten()
            .unwrap_or(false);
        let global_use_litellm_proxy = self.litellm_settings.use_litellm_proxy.unwrap_or(false);
        let deployment_use_litellm_proxy = self
            .model_list
            .into_iter()
            .map(|deployment| deployment.litellm_params.use_litellm_proxy.unwrap_or(false))
            .collect();
        Ok(Settings {
            api_base,
            api_key,
            environment_use_litellm_proxy,
            global_use_litellm_proxy,
            deployment_use_litellm_proxy,
        })
    }
}
