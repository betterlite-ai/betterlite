mod tests;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

// transcription's provider branches read the litellm module globals, never a model deployment
#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub organization: Option<String>,
    pub drop_params: Option<bool>,
}

// The globals and environment every base/key/token/organization fallback chain reads.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub organization: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub openai_organization: Option<String>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    // loaded so the Azure key chain can be shown to skip it, as LiteLLM's does
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub drop_params: Option<bool>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut settings = Settings {
            api_base: optional_string(self.settings.api_base, "litellm_settings.api_base", lookup)?,
            api_version: optional_string(
                self.settings.api_version,
                "litellm_settings.api_version",
                lookup,
            )?,
            api_key: resolve_secret(self.settings.api_key, "litellm_settings.api_key", lookup)?,
            openai_key: resolve_secret(
                self.settings.openai_key,
                "litellm_settings.openai_key",
                lookup,
            )?,
            azure_key: resolve_secret(
                self.settings.azure_key,
                "litellm_settings.azure_key",
                lookup,
            )?,
            organization: optional_string(
                self.settings.organization,
                "litellm_settings.organization",
                lookup,
            )?,
            drop_params: self.settings.drop_params,
            ..Self::Settings::default()
        };
        settings.apply_environment(lookup);
        Ok(settings)
    }
}

impl Settings {
    fn apply_environment<L>(&mut self, lookup: &L)
    where
        L: EnvironmentLookup,
    {
        self.azure_ad_token = lookup.lookup("AZURE_AD_TOKEN").map(SecretString::from);
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
