use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

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
    azure_account_url: Option<String>,
    azure_blob_container: Option<String>,
}

pub type Settings = Option<AzureBlobSettings>;

#[derive(Debug, Clone)]
pub struct AzureBlobSettings {
    pub account_url: String,
    pub container: String,
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
        if params.cache_type.as_deref() != Some("azure-blob") {
            return Ok(None);
        }
        convert_params(params, lookup).map(Some)
    }
}

fn convert_params<L>(params: CacheParams, lookup: &L) -> Result<AzureBlobSettings, ConfigError>
where
    L: EnvironmentLookup,
{
    let raw_account_url = params
        .azure_account_url
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "azure_account_url".to_owned(),
            path: "litellm_settings.cache_params.azure_account_url".to_owned(),
        })?;
    let account_url = resolve_string(
        raw_account_url,
        "litellm_settings.cache_params.azure_account_url",
        lookup,
    )?;
    let raw_container = params
        .azure_blob_container
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "azure_blob_container".to_owned(),
            path: "litellm_settings.cache_params.azure_blob_container".to_owned(),
        })?;
    let container = resolve_string(
        raw_container,
        "litellm_settings.cache_params.azure_blob_container",
        lookup,
    )?;
    Ok(AzureBlobSettings {
        account_url,
        container,
    })
}

mod tests;
