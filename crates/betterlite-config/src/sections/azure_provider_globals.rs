use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

pub const API_BASE_VARIABLE: &str = "AZURE_API_BASE";
pub const API_VERSION_VARIABLE: &str = "AZURE_API_VERSION";
pub const API_KEY_VARIABLE: &str = "AZURE_API_KEY";
pub const OPENAI_API_KEY_VARIABLE: &str = "AZURE_OPENAI_API_KEY";
pub const AD_TOKEN_VARIABLE: &str = "AZURE_AD_TOKEN";
pub const API_TYPE_VARIABLE: &str = "AZURE_API_TYPE";
const VARIABLES: [&str; 6] = [
    API_BASE_VARIABLE,
    API_KEY_VARIABLE,
    API_VERSION_VARIABLE,
    OPENAI_API_KEY_VARIABLE,
    AD_TOKEN_VARIABLE,
    API_TYPE_VARIABLE,
];

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: Params,
}

#[derive(Debug, Default, Deserialize)]
struct Params {
    api_base: Option<String>,
    api_version: Option<String>,
    api_key: Option<SecretString>,
    azure_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalSettings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub headers: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub globals: GlobalSettings,
    pub environment: BTreeMap<String, String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let params = self.litellm_settings;
        let path = "litellm_settings";
        let globals = GlobalSettings {
            api_base: resolve_text(params.api_base, &format!("{path}.api_base"), lookup)?,
            api_version: resolve_text(params.api_version, &format!("{path}.api_version"), lookup)?,
            api_key: resolve_secret(params.api_key, &format!("{path}.api_key"), lookup)?,
            azure_key: resolve_secret(params.azure_key, &format!("{path}.azure_key"), lookup)?,
            headers: resolve_headers(params.headers, lookup)?,
        };
        Ok(Settings {
            globals,
            environment: environment(lookup),
        })
    }
}

// LiteLLM reads each Azure variable with get_secret or get_secret_str, where an empty value counts as unset
fn environment<L>(lookup: &L) -> BTreeMap<String, String>
where
    L: EnvironmentLookup,
{
    let mut environment = BTreeMap::new();
    for name in VARIABLES {
        let value = lookup.lookup(name).filter(|text| !text.is_empty());
        if let Some(text) = value {
            environment.insert(name.to_owned(), text);
        }
    }
    environment
}

fn resolve_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let resolved = resolve_string(text, path, lookup)?;
    Ok(Some(resolved))
}

// litellm.headers is a mapping of strings, so each value resolves its own os.environ reference
fn resolve_headers<L>(
    given: Option<BTreeMap<String, String>>,
    lookup: &L,
) -> Result<Option<BTreeMap<String, String>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(headers) = given else {
        return Ok(None);
    };
    let mut resolved = BTreeMap::new();
    for (name, value) in headers {
        let text = resolve_string(value, "litellm_settings.headers", lookup)?;
        resolved.insert(name, text);
    }
    Ok(Some(resolved))
}

mod tests;
