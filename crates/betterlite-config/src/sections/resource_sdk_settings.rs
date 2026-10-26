mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
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

struct ConfiguredSettings {
    api_base: Option<String>,
    api_version: Option<String>,
    api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
    azure_key: Option<SecretString>,
    azure_ad_token: Option<SecretString>,
    organization: Option<String>,
    vertex_project: Option<String>,
    vertex_location: Option<String>,
    vertex_credentials: Option<SecretString>,
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
        let configured = self.settings.convert(lookup)?;
        let mut settings = Settings::from_configured(configured);
        settings.apply_environment(lookup);
        Ok(settings)
    }
}

impl RawSettings {
    fn convert<L>(self, lookup: &L) -> Result<ConfiguredSettings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = ConfiguredSettings {
            api_base: optional_string(self.api_base, "litellm_settings.api_base", lookup)?,
            api_version: optional_string(self.api_version, "litellm_settings.api_version", lookup)?,
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
        };
        Ok(configured)
    }
}

impl Settings {
    fn from_configured(configured: ConfiguredSettings) -> Self {
        let settings = Self {
            api_base: configured.api_base,
            api_version: configured.api_version,
            api_key: configured.api_key,
            openai_key: configured.openai_key,
            azure_key: configured.azure_key,
            azure_ad_token: configured.azure_ad_token,
            organization: configured.organization,
            vertex_project: configured.vertex_project,
            vertex_location: configured.vertex_location,
            vertex_credentials: configured.vertex_credentials,
            openai_base_url: None,
            openai_api_base: None,
            openai_api_key: None,
            openai_organization: None,
            azure_api_base: None,
            azure_api_version: None,
            azure_openai_api_key: None,
            azure_api_key: None,
            vertexai_project: None,
            vertexai_location: None,
            vertexai_credentials: None,
        };
        settings
    }

    fn apply_environment<L>(&mut self, lookup: &L)
    where
        L: EnvironmentLookup,
    {
        if self.azure_ad_token.is_none() {
            self.azure_ad_token = lookup.lookup("AZURE_AD_TOKEN").map(SecretString::from);
        }
        self.openai_base_url = lookup.lookup("OPENAI_BASE_URL");
        self.openai_api_base = lookup.lookup("OPENAI_API_BASE");
        self.openai_api_key = lookup.lookup("OPENAI_API_KEY").map(SecretString::from);
        self.openai_organization = lookup.lookup("OPENAI_ORGANIZATION");
        self.azure_api_base = lookup.lookup("AZURE_API_BASE");
        self.azure_api_version = lookup.lookup("AZURE_API_VERSION");
        self.azure_openai_api_key = lookup
            .lookup("AZURE_OPENAI_API_KEY")
            .map(SecretString::from);
        self.azure_api_key = lookup.lookup("AZURE_API_KEY").map(SecretString::from);
        self.vertexai_project = lookup.lookup("VERTEXAI_PROJECT");
        self.vertexai_location = lookup.lookup("VERTEXAI_LOCATION");
        self.vertexai_credentials = lookup
            .lookup("VERTEXAI_CREDENTIALS")
            .map(SecretString::from);
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
    match value {
        Some(text) => Ok(Some(resolve_string(text, path, lookup)?)),
        None => Ok(None),
    }
}
