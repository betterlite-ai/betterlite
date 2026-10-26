mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<String>,
    pub openai_key: Option<String>,
    pub azure_key: Option<String>,
    pub azure_ad_token: Option<String>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub openai_organization: Option<String>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub vertexai_project: Option<String>,
    pub vertexai_location: Option<String>,
    pub vertexai_credentials: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: optional_string(self.api_base, "litellm_settings.api_base", lookup)?,
            api_version: optional_string(
                self.api_version,
                "litellm_settings.api_version",
                lookup,
            )?,
            api_key: resolve_secret(self.api_key, "litellm_settings.api_key", lookup)?,
            openai_key: resolve_secret(self.openai_key, "litellm_settings.openai_key", lookup)?,
            azure_key: resolve_secret(self.azure_key, "litellm_settings.azure_key", lookup)?,
            azure_ad_token: resolve_secret(
                self.azure_ad_token,
                "litellm_settings.azure_ad_token",
                lookup,
            )?,
            organization: optional_string(
                self.organization,
                "litellm_settings.organization",
                lookup,
            )?,
            vertex_project: optional_string(
                self.vertex_project,
                "litellm_settings.vertex_project",
                lookup,
            )?,
            vertex_location: optional_string(
                self.vertex_location,
                "litellm_settings.vertex_location",
                lookup,
            )?,
            vertex_credentials: resolve_secret(
                self.vertex_credentials,
                "litellm_settings.vertex_credentials",
                lookup,
            )?,
            openai_base_url: lookup.lookup("OPENAI_BASE_URL"),
            openai_api_base: lookup.lookup("OPENAI_API_BASE"),
            openai_api_key: secret_environment(lookup, "OPENAI_API_KEY"),
            openai_organization: lookup.lookup("OPENAI_ORGANIZATION"),
            azure_api_base: lookup.lookup("AZURE_API_BASE"),
            azure_api_version: lookup.lookup("AZURE_API_VERSION"),
            azure_openai_api_key: secret_environment(lookup, "AZURE_OPENAI_API_KEY"),
            azure_api_key: secret_environment(lookup, "AZURE_API_KEY"),
            vertexai_project: lookup.lookup("VERTEXAI_PROJECT"),
            vertexai_location: lookup.lookup("VERTEXAI_LOCATION"),
            vertexai_credentials: secret_environment(lookup, "VERTEXAI_CREDENTIALS"),
        })
    }
}

fn optional_string<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    value
        .map(|text| resolve_string(text, path, lookup))
        .transpose()
}

fn secret_environment<L>(lookup: &L, name: &str) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    lookup.lookup(name).map(SecretString::from)
}
