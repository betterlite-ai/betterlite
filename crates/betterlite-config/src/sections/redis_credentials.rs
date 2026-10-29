use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::{resolve_secret, resolve_string};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    #[serde(default)]
    cache_params: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    gcp_service_account: Option<String>,
    gcp_ssl_ca_certs: Option<String>,
    azure_redis_ad_token: Option<String>,
    azure_client_id: Option<String>,
    azure_tenant_id: Option<String>,
    azure_client_secret: Option<SecretString>,
    aws_iam_auth: Option<ScalarBool>,
    aws_iam_user_name: Option<String>,
    aws_iam_cache_name: Option<String>,
    aws_iam_region: Option<String>,
    aws_iam_serverless: Option<ScalarBool>,
    ssl: Option<ScalarBool>,
    url: Option<SecretString>,
    username: Option<String>,
    password: Option<SecretString>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ScalarBool {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone)]
pub enum Provider {
    Gcp(GcpSettings),
    Azure(AzureSettings),
    Aws(AwsSettings),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderWarning {
    GcpOutranksAzure,
    GcpOutranksAws,
    AzureOutranksAws,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub provider: Option<Provider>,
    pub url: Option<SecretString>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub warnings: Vec<ProviderWarning>,
    pub environment_snapshot: BTreeMap<String, bool>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.litellm_settings.cache_params;
        let environment_snapshot = environment_snapshot(lookup);
        let (provider, warnings) = resolve_providers(&settings, lookup)?;
        let (url, username, password) = resolve_connection(&settings, lookup, provider.as_ref())?;

        Ok(Settings {
            provider,
            url,
            username,
            password,
            warnings,
            environment_snapshot,
        })
    }
}

type ConnectionFields = (Option<SecretString>, Option<String>, Option<SecretString>);

fn resolve_connection<L>(
    settings: &RawSettings,
    lookup: &L,
    provider: Option<&Provider>,
) -> Result<ConnectionFields, ConfigError>
where
    L: EnvironmentLookup,
{
    let configured_url = resolve_secret_opt(
        settings
            .url
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        "litellm_settings.cache_params.url",
        lookup,
        "REDIS_URL",
    )?;
    Ok(match provider {
        Some(Provider::Azure(azure)) => {
            (stripped_url(configured_url)?, azure.username.clone(), None)
        }
        Some(_) => (stripped_url(configured_url)?, None, None),
        None => {
            let username = resolve_string_opt(
                settings.username.clone(),
                "litellm_settings.cache_params.username",
                lookup,
            )?
            .or_else(|| lookup.lookup("REDIS_USERNAME"));
            let password = resolve_secret_opt(
                settings
                    .password
                    .as_ref()
                    .map(|secret| secret.expose_secret().to_owned()),
                "litellm_settings.cache_params.password",
                lookup,
                "REDIS_PASSWORD",
            )?;
            (configured_url, username, password)
        }
    })
}

pub(super) fn resolve_secret_opt<L>(
    configured: Option<String>,
    path: &str,
    lookup: &L,
    env_name: &str,
) -> Result<Option<SecretString>, ConfigError>
where
    L: EnvironmentLookup,
{
    let resolved = resolve_secret(configured, path, lookup)?;
    Ok(resolved.or_else(|| lookup.lookup(env_name).map(SecretString::from)))
}

fn resolve_string_opt<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    resolve_string(text, path, lookup).map(Some)
}

fn environment_snapshot<L>(lookup: &L) -> BTreeMap<String, bool>
where
    L: EnvironmentLookup,
{
    let names = [
        "REDIS_GCP_SERVICE_ACCOUNT",
        "REDIS_GCP_SSL_CA_CERTS",
        "REDIS_AZURE_AD_TOKEN",
        "REDIS_AZURE_CLIENT_ID",
        "REDIS_AZURE_TENANT_ID",
        "REDIS_AZURE_CLIENT_SECRET",
        "REDIS_AWS_IAM_AUTH",
        "REDIS_AWS_IAM_USER_NAME",
        "REDIS_AWS_IAM_CACHE_NAME",
        "REDIS_AWS_IAM_REGION",
        "REDIS_AWS_IAM_SERVERLESS",
        "REDIS_SSL",
        "REDIS_URL",
        "REDIS_USERNAME",
        "REDIS_PASSWORD",
        "AZURE_CLIENT_ID",
        "AZURE_TENANT_ID",
        "AZURE_CLIENT_SECRET",
        "AWS_REGION",
        "AWS_DEFAULT_REGION",
    ];
    names
        .into_iter()
        .map(|name| (name.to_owned(), lookup.lookup(name).is_some()))
        .collect()
}

fn config_or_environment_flag<L>(
    configured: Option<&ScalarBool>,
    lookup: &L,
    variable: &str,
) -> bool
where
    L: EnvironmentLookup,
{
    let Some(value) = configured else {
        return lookup
            .lookup(variable)
            .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)));
    };
    coerces_to_true(value)
}

fn non_empty(value: Option<String>) -> Option<String> {
    match value {
        Some(text) if !text.is_empty() => Some(text),
        _ => None,
    }
}

fn coerces_to_true(setting: &ScalarBool) -> bool {
    match setting {
        ScalarBool::Boolean(boolean) => *boolean,
        ScalarBool::Text(text) => {
            matches!(text.to_ascii_lowercase().as_str(), "true" | "1" | "yes")
        }
    }
}

fn stripped_url(configured_url: Option<SecretString>) -> Result<Option<SecretString>, ConfigError> {
    let Some(value) = configured_url else {
        return Ok(None);
    };
    url_without_userinfo(&value).map(Some)
}

fn url_without_userinfo(url: &SecretString) -> Result<SecretString, ConfigError> {
    let mut parsed = url::Url::parse(url.expose_secret()).map_err(|_parse_error| {
        ConfigError::InvalidScalar {
            path: "litellm_settings.cache_params.url".to_owned(),
            value: false,
        }
    })?;
    parsed
        .set_username("")
        .map_err(|()| ConfigError::InvalidScalar {
            path: "litellm_settings.cache_params.url".to_owned(),
            value: false,
        })?;
    parsed
        .set_password(None)
        .map_err(|()| ConfigError::InvalidScalar {
            path: "litellm_settings.cache_params.url".to_owned(),
            value: false,
        })?;
    Ok(SecretString::from(parsed.to_string()))
}

#[path = "redis_credentials/aws.rs"]
pub mod aws;
use aws::AwsSettings;

#[path = "redis_credentials/azure.rs"]
pub mod azure;
use azure::AzureSettings;

#[path = "redis_credentials/gcp.rs"]
pub mod gcp;
use gcp::GcpSettings;

#[path = "redis_credentials/provider.rs"]
mod provider;
use provider::resolve_providers;

#[path = "redis_credentials/tests.rs"]
mod tests;
