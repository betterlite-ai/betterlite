use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

mod tests;

pub type Headers = BTreeMap<String, String>;

// main.py's Groq, Cloudflare, Fireworks and Together arms: the SDK globals through litellm, the rest through get_secret_str
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
    groq_key: Option<SecretString>,
    cloudflare_api_key: Option<SecretString>,
}

// The provider variables each arm falls back to last; an empty one counts as unset
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub groq_api_key: Option<SecretString>,
    pub groq_api_base: Option<String>,
    pub cloudflare_api_key: Option<SecretString>,
    pub cloudflare_api_base: Option<String>,
    pub cloudflare_account_id: Option<String>,
    pub fireworks_ai_api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub groq_key: Option<SecretString>,
    pub cloudflare_api_key: Option<SecretString>,
    pub environment: Environment,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // get_secret_str reads the provider variables; an empty one counts as unset
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
            groq_key: resolve_secret(
                self.litellm_settings.groq_key,
                "litellm_settings.groq_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            cloudflare_api_key: resolve_secret(
                self.litellm_settings.cloudflare_api_key,
                "litellm_settings.cloudflare_api_key",
                lookup,
            )?
            .filter(|value| !value.expose_secret().is_empty()),
            environment: Environment {
                groq_api_key: read("GROQ_API_KEY").map(SecretString::from),
                groq_api_base: read("GROQ_API_BASE"),
                cloudflare_api_key: read("CLOUDFLARE_API_KEY").map(SecretString::from),
                cloudflare_api_base: read("CLOUDFLARE_API_BASE"),
                cloudflare_account_id: read("CLOUDFLARE_ACCOUNT_ID"),
                fireworks_ai_api_key: read("FIREWORKS_AI_API_KEY").map(SecretString::from),
            },
        })
    }
}
