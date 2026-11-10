mod tests;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_API_BASE: &str = "https://api.anthropic.com";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
    #[serde(default)]
    model_list: Vec<Deployment>,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    use_bearer_for_custom_base: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct Deployment {
    #[serde(default)]
    litellm_params: Params,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub api_key: Option<SecretString>,
    pub auth_token: Option<SecretString>,
    pub api_base: String,
    pub use_bearer_for_custom_base: bool,
    pub deployment_bearer_opt_ins: Vec<bool>,
    pub disable_url_suffix: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: None,
            auth_token: None,
            api_base: DEFAULT_API_BASE.to_owned(),
            use_bearer_for_custom_base: false,
            deployment_bearer_opt_ins: Vec::new(),
            disable_url_suffix: false,
        }
    }
}

impl Settings {
    pub fn resolve_environment<L>(lookup: &L) -> Result<Self, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let disable_url_suffix = lookup
            .lookup("LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX")
            .map(|text| betterlite_env::flag("LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX", &text))
            .transpose()?
            .flatten()
            .unwrap_or(false);
        Ok(Self {
            // LiteLLM's get_secret_str keeps an empty value as a real key, so only absence is unset
            api_key: lookup.lookup("ANTHROPIC_API_KEY").map(SecretString::from),
            auth_token: lookup
                .lookup("ANTHROPIC_AUTH_TOKEN")
                .map(SecretString::from),
            api_base: api_base_from(lookup),
            use_bearer_for_custom_base: false,
            deployment_bearer_opt_ins: Vec::new(),
            disable_url_suffix,
        })
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut settings = Settings::resolve_environment(lookup)?;
        let module_bearer = self
            .litellm_settings
            .use_bearer_for_custom_base
            .unwrap_or(false);
        let deployment_bearer_opt_ins = self
            .model_list
            .into_iter()
            .map(|deployment| {
                deployment
                    .litellm_params
                    .use_bearer_for_custom_base
                    .unwrap_or(module_bearer)
            })
            .collect();
        settings.use_bearer_for_custom_base = module_bearer;
        settings.deployment_bearer_opt_ins = deployment_bearer_opt_ins;
        Ok(settings)
    }
}

fn api_base_from<L>(lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    non_empty(lookup.lookup("ANTHROPIC_API_BASE"))
        .or_else(|| non_empty(lookup.lookup("ANTHROPIC_BASE_URL")))
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
}

fn non_empty(value: Option<String>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        Some(_) | None => None,
    }
}
