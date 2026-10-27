use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::Section;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};

pub mod options;
use options::Options;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    litellm_settings: LiteLlmSettings,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LiteLlmSettings {
    cache_params: Option<CacheParams>,
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
    #[serde(rename = "s3_use_ssl")]
    use_ssl: Option<bool>,
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
    config: Options,
    #[serde(rename = "s3_path")]
    path: Option<String>,
}

pub type Settings = Option<S3Settings>;

#[derive(Debug, Clone)]
pub struct S3Settings {
    pub bucket_name: String,
    pub region_name: String,
    pub endpoint_url: Option<String>,
    pub credentials: Option<Credentials>,
    pub config: Options,
    pub key_prefix: String,
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub access_key: SecretString,
    pub secret_key: SecretString,
    pub session_token: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        self.litellm_settings
            .cache_params
            .map(|params| convert_params(params, lookup))
            .transpose()
    }
}

fn convert_params<L>(params: CacheParams, lookup: &L) -> Result<S3Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    validate(&params)?;
    let bucket = params
        .bucket_name
        .ok_or_else(|| ConfigError::MissingEnvironment {
            variable: "s3_bucket_name".to_owned(),
            path: "litellm_settings.cache_params.s3_bucket_name".to_owned(),
        })?;
    let bucket_name = resolve_string(
        bucket,
        "litellm_settings.cache_params.s3_bucket_name",
        lookup,
    )?;
    let credentials = credentials(
        params.aws_access_key_id,
        params.aws_secret_access_key,
        params.aws_session_token,
        lookup,
    )?;
    let key_prefix = params.path.as_deref().filter(|path| !path.is_empty())
        .map_or_else(String::new, |path| format!("{}/", path.trim_end_matches('/')));
    Ok(S3Settings {
        bucket_name,
        region_name: params.region_name.unwrap_or_else(|| "us-east-1".to_owned()),
        endpoint_url: params.endpoint_url,
        credentials,
        config: params.config,
        key_prefix,
    })
}

fn validate(params: &CacheParams) -> Result<(), ConfigError> {
    let mut paths = Vec::new();
    if !params.use_ssl.unwrap_or(true) {
        paths.push("litellm_settings.cache_params.s3_use_ssl".to_owned());
    }
    if params.verify == Some(false) {
        paths.push("litellm_settings.cache_params.s3_verify".to_owned());
    }
    if params
        .api_version
        .as_deref()
        .is_some_and(|version| version != "2006-03-01")
    {
        paths.push("litellm_settings.cache_params.s3_api_version".to_owned());
    }
    params.config.validate(&mut paths);
    if paths.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::Unsupported { paths })
    }
}

fn credentials<L>(
    access: Option<SecretString>,
    secret: Option<SecretString>,
    token: Option<SecretString>,
    lookup: &L,
) -> Result<Option<Credentials>, ConfigError>
where
    L: EnvironmentLookup,
{
    match (access, secret, token) {
        (None, None, None) => Ok(None),
        (Some(access_key), Some(secret_key), session_token) => Ok(Some(Credentials {
            access_key: SecretString::from(resolve_string(access_key.expose_secret().to_owned(),
                "s3_aws_access_key_id", lookup)?),
            secret_key: SecretString::from(resolve_string(secret_key.expose_secret().to_owned(),
                "s3_aws_secret_access_key", lookup)?),
            session_token: match session_token {
                None => None,
                Some(value) => resolve_secret(Some(value.expose_secret().to_owned()),
                    "s3_aws_session_token", lookup)?,
            },
        })),
        _ => Err(ConfigError::Unsupported { paths: vec![
            "s3_aws_access_key_id/s3_aws_secret_access_key/s3_aws_session_token (partial credentials)".to_owned(),
        ] }),
    }
}

mod tests;
