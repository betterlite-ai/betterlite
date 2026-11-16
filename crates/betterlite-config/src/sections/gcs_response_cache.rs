use secrecy::SecretString;
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

const BUCKET_PATH: &str = "litellm_settings.cache_params.gcs_bucket_name";
const SERVICE_ACCOUNT_PATH: &str = "litellm_settings.cache_params.gcs_path_service_account";
const PATH_PATH: &str = "litellm_settings.cache_params.gcs_path";
const BUCKET_VARIABLE: &str = "GCS_BUCKET_NAME";
const SERVICE_ACCOUNT_VARIABLE: &str = "GCS_PATH_SERVICE_ACCOUNT";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    litellm_settings: LiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LiteLlmSettings {
    cache: Option<bool>,
    cache_params: Option<CacheParams>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CacheParams {
    #[serde(rename = "type")]
    cache_type: Option<String>,
    gcs_bucket_name: Option<String>,
    gcs_path_service_account: Option<String>,
    gcs_path: Option<String>,
}

pub type Settings = Option<GcsResponseCacheSettings>;

#[derive(Debug, Clone)]
pub struct GcsResponseCacheSettings {
    pub bucket_name: String,
    pub service_account_path: Option<SecretString>,
    pub path: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        if self.litellm_settings.cache != Some(true) {
            return Ok(None);
        }
        let Some(params) = self.litellm_settings.cache_params else {
            return Ok(None);
        };
        if params.cache_type.as_deref() != Some("gcs") {
            return Ok(None);
        }
        convert_params(params, lookup).map(Some)
    }
}

fn convert_params<L>(
    params: CacheParams,
    lookup: &L,
) -> Result<GcsResponseCacheSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let bucket_name = resolve_bucket_name(params.gcs_bucket_name, lookup)?;
    let service_account_path = resolve_service_account(params.gcs_path_service_account, lookup)?;
    let path = match params.gcs_path {
        Some(raw) => resolve_string(raw, PATH_PATH, lookup)?,
        None => String::new(),
    };
    Ok(GcsResponseCacheSettings {
        bucket_name,
        service_account_path,
        path,
    })
}

fn resolve_bucket_name<L>(raw: Option<String>, lookup: &L) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = match raw {
        Some(value) => resolve_string(value, BUCKET_PATH, lookup)?,
        None => String::new(),
    };
    if !configured.is_empty() {
        return Ok(configured);
    }
    lookup
        .lookup(BUCKET_VARIABLE)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: BUCKET_VARIABLE.to_owned(),
            path: BUCKET_PATH.to_owned(),
        })
}

fn resolve_service_account<L>(
    raw: Option<String>,
    lookup: &L,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured = match raw {
        Some(value) => resolve_string(value, SERVICE_ACCOUNT_PATH, lookup)?,
        None => String::new(),
    };
    if !configured.is_empty() {
        return Ok(Some(SecretString::from(configured)));
    }
    Ok(lookup
        .lookup(SERVICE_ACCOUNT_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(SecretString::from))
}

mod tests;
