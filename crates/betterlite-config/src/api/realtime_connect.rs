mod tests;

use betterlite_env::number;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, ConfigLoader, EnvironmentLookup};

pub const CREDENTIAL_TIMEOUT_VARIABLE: &str = "REALTIME_CREDENTIAL_RESOLUTION_TIMEOUT_SECONDS";
pub const AZURE_PROTOCOL_VARIABLE: &str = "LITELLM_AZURE_REALTIME_PROTOCOL";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 20.0;

// The globals and environment-backed fallbacks _arealtime reads before it hands a branch its credentials.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RealtimeConnectionSettings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub xai_key: Option<SecretString>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub azure_api_base: Option<String>,
    pub azure_api_key: Option<SecretString>,
    pub openai_api_key: Option<SecretString>,
    pub xai_api_base: Option<String>,
    pub xai_api_key: Option<SecretString>,
    pub vertexai_credentials: Option<Json>,
    pub vertexai_project: Option<String>,
    pub vertexai_location: Option<String>,
    pub azure_realtime_protocol: Option<String>,
    #[serde(default = "default_credential_timeout")]
    pub credential_timeout_seconds: f64,
}

impl Default for RealtimeConnectionSettings {
    fn default() -> Self {
        Self {
            api_base: None,
            api_key: None,
            openai_key: None,
            xai_key: None,
            vertex_project: None,
            vertex_location: None,
            azure_api_base: None,
            azure_api_key: None,
            openai_api_key: None,
            xai_api_base: None,
            xai_api_key: None,
            vertexai_credentials: None,
            vertexai_project: None,
            vertexai_location: None,
            azure_realtime_protocol: None,
            credential_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

const fn default_credential_timeout() -> f64 {
    DEFAULT_TIMEOUT_SECONDS
}

// Loads the whole config, so an unsupported key fails here exactly as it does for every other section.
pub fn load_settings<L>(source: &str, lookup: L) -> Result<RealtimeConnectionSettings, ConfigError>
where
    L: EnvironmentLookup + Sync,
{
    let azure_realtime_protocol = lookup
        .lookup(AZURE_PROTOCOL_VARIABLE)
        .filter(|value| !value.is_empty());
    let credential_timeout_seconds = credential_timeout(&lookup)?;
    let config = ConfigLoader { lookup }.load(source)?;
    let resource = &config.sections.resource_sdk_settings;
    let xai = &config.sections.xai_identity;
    Ok(RealtimeConnectionSettings {
        api_base: resource.api_base.clone(),
        api_key: resource.api_key.clone(),
        openai_key: resource.openai_key.clone(),
        xai_key: xai.xai_key.clone(),
        vertex_project: resource.vertex_project.clone(),
        vertex_location: resource.vertex_location.clone(),
        azure_api_base: resource
            .azure_api_base
            .clone()
            .filter(|value| !value.is_empty()),
        azure_api_key: non_empty_secret(resource.azure_api_key.as_ref()),
        openai_api_key: non_empty_secret(resource.openai_api_key.as_ref()),
        xai_api_base: xai.api_base.clone(),
        xai_api_key: xai.environment_key.clone(),
        vertexai_credentials: resource
            .vertexai_credentials
            .as_ref()
            .filter(|value| !value.expose_secret().is_empty())
            .map(|value| Json::Text(value.expose_secret().to_owned())),
        vertexai_project: resource
            .vertexai_project
            .clone()
            .filter(|value| !value.is_empty()),
        vertexai_location: resource
            .vertexai_location
            .clone()
            .filter(|value| !value.is_empty()),
        azure_realtime_protocol,
        credential_timeout_seconds,
    })
}

// the constants module parses float(os.getenv(name, "20.0")), so an empty value is the default
fn credential_timeout<L>(lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup + ?Sized,
{
    let Some(text) = lookup.lookup(CREDENTIAL_TIMEOUT_VARIABLE) else {
        return Ok(DEFAULT_TIMEOUT_SECONDS);
    };
    Ok(number::<f64>(CREDENTIAL_TIMEOUT_VARIABLE, &text)?.unwrap_or(DEFAULT_TIMEOUT_SECONDS))
}

fn non_empty_secret(value: Option<&SecretString>) -> Option<SecretString> {
    value
        .filter(|value| !value.expose_secret().is_empty())
        .cloned()
}
