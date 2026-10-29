use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
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
    url: Option<String>,
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
        let (url, username, password) = resolve_connection(&settings, lookup, provider.as_ref());

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

fn resolve_connection<L>(
    settings: &RawSettings,
    lookup: &L,
    provider: Option<&Provider>,
) -> (Option<SecretString>, Option<String>, Option<SecretString>)
where
    L: EnvironmentLookup,
{
    let configured_url = settings.url.clone().or_else(|| lookup.lookup("REDIS_URL"));
    match provider {
        Some(Provider::Azure(azure)) => (
            configured_url.map(|value| SecretString::from(url_without_userinfo(&value))),
            azure.username.clone(),
            None,
        ),
        Some(_) => (
            configured_url.map(|value| SecretString::from(url_without_userinfo(&value))),
            None,
            None,
        ),
        None => {
            let username = settings
                .username
                .clone()
                .or_else(|| lookup.lookup("REDIS_USERNAME"));
            let password = settings
                .password
                .clone()
                .or_else(|| lookup.lookup("REDIS_PASSWORD").map(SecretString::from));
            (configured_url.map(SecretString::from), username, password)
        }
    }
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

fn ssl_enabled<L>(settings: &RawSettings, lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    let configured = settings.ssl.as_ref();
    configured.map_or_else(
        || {
            lookup
                .lookup("REDIS_SSL")
                .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)))
        },
        coerces_to_true,
    )
}

fn coerces_to_true(setting: &ScalarBool) -> bool {
    match setting {
        ScalarBool::Boolean(boolean) => *boolean,
        ScalarBool::Text(text) => {
            matches!(text.to_ascii_lowercase().as_str(), "true" | "1" | "yes")
        }
    }
}

fn url_without_userinfo(url: &str) -> String {
    let (scheme, rest) = url
        .split_once("://")
        .map_or((None, url), |(url_scheme, remainder)| {
            (Some(url_scheme), remainder)
        });
    let netloc_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (netloc_with_userinfo, tail) = rest.split_at(netloc_end);
    let netloc = netloc_with_userinfo
        .rsplit_once('@')
        .map_or(netloc_with_userinfo, |(_, host)| host);
    scheme.map_or_else(
        || format!("{netloc}{tail}"),
        |url_scheme| format!("{url_scheme}://{netloc}{tail}"),
    )
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
