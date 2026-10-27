use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    litellm_settings: LiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LiteLlmSettings {
    cache_params: CacheParams,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CacheParams {
    #[serde(rename = "s3_bucket_name")]
    bucket_name: Option<String>,
    #[serde(rename = "s3_region_name")]
    region_name: Option<String>,
    #[serde(rename = "s3_api_version")]
    api_version: Option<String>,
    #[serde(rename = "s3_use_ssl", default = "default_use_ssl")]
    use_ssl: bool,
    #[serde(rename = "s3_verify")]
    verify: Option<bool>,
    #[serde(rename = "s3_endpoint_url")]
    endpoint_url: Option<String>,
    #[serde(rename = "s3_aws_access_key_id")]
    aws_access_key_id: Option<SecretString>,
    #[serde(rename = "s3_aws_secret_access_key")]
    aws_secret_access_key: Option<SecretString>,
    #[serde(rename = "s3_aws_session_token")]
    aws_session_token: Option<SecretString>,
    #[serde(rename = "s3_config")]
    config: Option<BTreeMap<String, String>>,
    #[serde(rename = "s3_path")]
    path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub bucket_name: String,
    pub region_name: Option<String>,
    pub api_version: Option<String>,
    pub use_ssl: bool,
    pub verify: Option<bool>,
    pub endpoint_url: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub config: Option<BTreeMap<String, String>>,
    pub path: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let params = self.litellm_settings.cache_params;
        let bucket_name = params
            .bucket_name
            .ok_or_else(|| ConfigError::MissingEnvironment {
                variable: "s3_bucket_name".to_owned(),
                path: "litellm_settings.cache_params.s3_bucket_name".to_owned(),
            })
            .and_then(|value| {
                resolve_string(
                    value,
                    "litellm_settings.cache_params.s3_bucket_name",
                    lookup,
                )
            })?;
        Ok(Settings {
            bucket_name,
            region_name: params.region_name,
            api_version: params.api_version,
            use_ssl: params.use_ssl,
            verify: params.verify,
            endpoint_url: params.endpoint_url,
            aws_access_key_id: resolve_optional_secret(
                params.aws_access_key_id,
                "litellm_settings.cache_params.s3_aws_access_key_id",
                lookup,
            )?,
            aws_secret_access_key: resolve_optional_secret(
                params.aws_secret_access_key,
                "litellm_settings.cache_params.s3_aws_secret_access_key",
                lookup,
            )?,
            aws_session_token: resolve_optional_secret(
                params.aws_session_token,
                "litellm_settings.cache_params.s3_aws_session_token",
                lookup,
            )?,
            config: params.config,
            path: params.path,
        })
    }
}

fn resolve_optional_secret<L>(
    value: Option<SecretString>,
    path: &str,
    lookup: &L,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    value
        .map(|secret| {
            resolve_secret(
                Some(secret.expose_secret().to_owned()),
                path,
                lookup,
            )
        })
        .transpose()
        .map(Option::flatten)
}

fn default_use_ssl() -> bool {
    let enabled = true;
    enabled
}

#[cfg(test)]
#[path = "s3_response_cache/tests.rs"]
mod tests;
