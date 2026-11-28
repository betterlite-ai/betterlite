mod tests;

use std::collections::BTreeMap;

use secrecy::SecretString;

use crate::env_integer::timeout::RequestTimeout;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_integer, resolve_number, resolve_secret, resolve_string};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    settings: RawSettings,
}

// every litellm_settings key speech reads as a module global, plus the two numbers it resolves
#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    openai_key: Option<SecretString>,
    organization: Option<String>,
    project: Option<String>,
    azure_key: Option<SecretString>,
    api_version: Option<String>,
    headers: Option<BTreeMap<String, String>>,
    num_retries: Option<RawNumber>,
    request_timeout: Option<RawNumber>,
}

// the litellm globals speech reads, with the environment tiers get_secret would resolve
#[derive(Debug, Clone)]
pub struct Settings {
    pub request_timeout_seconds: f64,
    pub num_retries: Option<u32>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub azure_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub headers: Option<BTreeMap<String, String>>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub openai_organization: Option<String>,
    pub openai_project: Option<String>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_environment(&BTreeMap::<String, String>::new())
    }
}

impl Settings {
    fn from_environment<L>(lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            request_timeout_seconds: RequestTimeout::DEFAULT_SECONDS,
            num_retries: None,
            api_base: None,
            api_key: None,
            openai_key: None,
            organization: None,
            project: None,
            azure_key: None,
            api_version: None,
            headers: None,
            openai_base_url: lookup
                .lookup("OPENAI_BASE_URL")
                .filter(|value| !value.is_empty()),
            openai_api_base: lookup
                .lookup("OPENAI_API_BASE")
                .filter(|value| !value.is_empty()),
            openai_api_key: environment_secret(lookup, "OPENAI_API_KEY"),
            openai_organization: lookup
                .lookup("OPENAI_ORGANIZATION")
                .filter(|value| !value.is_empty()),
            openai_project: lookup
                .lookup("OPENAI_PROJECT")
                .filter(|value| !value.is_empty()),
            azure_api_base: lookup
                .lookup("AZURE_API_BASE")
                .filter(|value| !value.is_empty()),
            azure_api_version: lookup
                .lookup("AZURE_API_VERSION")
                .filter(|value| !value.is_empty()),
            azure_openai_api_key: environment_secret(lookup, "AZURE_OPENAI_API_KEY"),
            azure_api_key: environment_secret(lookup, "AZURE_API_KEY"),
            azure_ad_token: environment_secret(lookup, "AZURE_AD_TOKEN"),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.settings;
        let mut resolved = Settings::from_environment(lookup);
        resolved.request_timeout_seconds =
            RequestTimeout::resolve_configured(settings.request_timeout, lookup)?.seconds;
        resolved.num_retries = resolve_number(
            settings.num_retries,
            "litellm_settings.num_retries",
            lookup,
            parse_integer,
        )?;
        resolved.api_base = resolved_text(settings.api_base, "litellm_settings.api_base", lookup)?;
        resolved.api_key = resolve_secret(settings.api_key, "litellm_settings.api_key", lookup)?;
        resolved.openai_key =
            resolve_secret(settings.openai_key, "litellm_settings.openai_key", lookup)?;
        resolved.organization = resolved_text(
            settings.organization,
            "litellm_settings.organization",
            lookup,
        )?;
        resolved.project = resolved_text(settings.project, "litellm_settings.project", lookup)?;
        resolved.azure_key =
            resolve_secret(settings.azure_key, "litellm_settings.azure_key", lookup)?;
        resolved.api_version =
            resolved_text(settings.api_version, "litellm_settings.api_version", lookup)?;
        resolved.headers = settings.headers;
        Ok(resolved)
    }
}

fn resolved_text<L>(
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

// get_secret treats an empty variable as unset, so the chain never sees ""
fn environment_secret<L>(lookup: &L, name: &str) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    lookup
        .lookup(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}
