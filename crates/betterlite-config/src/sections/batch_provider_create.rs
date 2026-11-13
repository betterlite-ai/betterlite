use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

mod tests;

const MODULE_PATH: &str = "litellm_settings";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: ModuleSettings,
}

#[derive(Debug, Default, Deserialize)]
struct ModuleSettings {
    api_base: Option<String>,
    organization: Option<String>,
    api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
    api_version: Option<String>,
    azure_key: Option<SecretString>,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
}

// Every value keeps its own key: create_batch picks the first one set for the provider it calls
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub organization: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub azure_key: Option<SecretString>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_organization: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub vertexai_project: Option<String>,
    pub vertexai_location: Option<String>,
    pub vertexai_credentials: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let module = self.litellm_settings;
        Ok(Settings {
            api_base: text(module.api_base, "api_base", lookup)?,
            organization: text(module.organization, "organization", lookup)?,
            api_key: secret(module.api_key, "api_key", lookup)?,
            openai_key: secret(module.openai_key, "openai_key", lookup)?,
            api_version: text(module.api_version, "api_version", lookup)?,
            azure_key: secret(module.azure_key, "azure_key", lookup)?,
            vertex_project: text(module.vertex_project, "vertex_project", lookup)?,
            vertex_location: text(module.vertex_location, "vertex_location", lookup)?,
            openai_base_url: environment(lookup, "OPENAI_BASE_URL"),
            openai_api_base: environment(lookup, "OPENAI_API_BASE"),
            openai_organization: environment(lookup, "OPENAI_ORGANIZATION"),
            openai_api_key: environment_secret(lookup, "OPENAI_API_KEY"),
            azure_api_base: environment(lookup, "AZURE_API_BASE"),
            azure_api_version: environment(lookup, "AZURE_API_VERSION"),
            azure_openai_api_key: environment_secret(lookup, "AZURE_OPENAI_API_KEY"),
            azure_api_key: environment_secret(lookup, "AZURE_API_KEY"),
            azure_ad_token: environment_secret(lookup, "AZURE_AD_TOKEN"),
            vertexai_project: environment(lookup, "VERTEXAI_PROJECT"),
            vertexai_location: environment(lookup, "VERTEXAI_LOCATION"),
            vertexai_credentials: environment_secret(lookup, "VERTEXAI_CREDENTIALS"),
        })
    }
}

// an os.environ/ reference resolves through the supplied environment, and an empty value counts as unset
fn text<L>(value: Option<String>, field: &str, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = value
        .map(|text| resolve_string(text, &format!("{MODULE_PATH}.{field}"), lookup))
        .transpose()?;
    Ok(resolved.filter(|text| !text.is_empty()))
}

fn secret<L>(
    value: Option<SecretString>,
    field: &str,
    lookup: &L,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = resolve_secret(value, &format!("{MODULE_PATH}.{field}"), lookup)?;
    Ok(resolved.filter(|secret| !secret.expose_secret().is_empty()))
}

fn environment<L>(lookup: &L, name: &str) -> Option<String>
where
    L: EnvironmentLookup,
{
    lookup.lookup(name).filter(|value| !value.is_empty())
}

fn environment_secret<L>(lookup: &L, name: &str) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    environment(lookup, name).map(SecretString::from)
}
