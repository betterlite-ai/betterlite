use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

mod tests;

pub type Headers = BTreeMap<String, String>;

// main.py's OpenRouter and Vercel AI Gateway arms: the SDK globals through litellm, the rest through get_secret
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    #[serde(default)]
    headers: Headers,
    openrouter_key: Option<SecretString>,
}

// The provider variables the two arms fall back to; an empty one counts as unset
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub openrouter_api_base: Option<String>,
    pub openrouter_api_key: Option<SecretString>,
    pub or_api_key: Option<SecretString>,
    pub or_site_url: Option<String>,
    pub or_app_name: Option<String>,
    pub vercel_ai_gateway_api_base: Option<String>,
    pub vercel_ai_gateway_api_key: Option<SecretString>,
    pub vercel_oidc_token: Option<SecretString>,
    pub vercel_site_url: Option<String>,
    pub vercel_app_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub openrouter_key: Option<SecretString>,
    pub environment: Environment,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // get_secret_str and get_secret read the provider variables; an empty one counts as unset
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let mut headers = Headers::new();
        for (name, value) in self.litellm_settings.headers {
            let resolved = resolve_string(value, "litellm_settings.headers", lookup)?;
            headers.insert(name, resolved);
        }
        Ok(Settings {
            api_base: match self.litellm_settings.api_base {
                Some(text) => Some(resolve_string(text, "litellm_settings.api_base", lookup)?),
                None => None,
            }
            .filter(|value| !value.is_empty()),
            api_key: resolve_secret(
                self.litellm_settings.api_key,
                "litellm_settings.api_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            headers: (!headers.is_empty()).then_some(headers),
            openrouter_key: resolve_secret(
                self.litellm_settings.openrouter_key,
                "litellm_settings.openrouter_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            environment: Environment {
                openrouter_api_base: read("OPENROUTER_API_BASE"),
                openrouter_api_key: read("OPENROUTER_API_KEY").map(SecretString::from),
                or_api_key: read("OR_API_KEY").map(SecretString::from),
                or_site_url: read("OR_SITE_URL"),
                or_app_name: read("OR_APP_NAME"),
                vercel_ai_gateway_api_base: read("VERCEL_AI_GATEWAY_API_BASE"),
                vercel_ai_gateway_api_key: read("VERCEL_AI_GATEWAY_API_KEY")
                    .map(SecretString::from),
                vercel_oidc_token: read("VERCEL_OIDC_TOKEN").map(SecretString::from),
                vercel_site_url: read("VERCEL_SITE_URL"),
                vercel_app_name: read("VERCEL_APP_NAME"),
            },
        })
    }
}
