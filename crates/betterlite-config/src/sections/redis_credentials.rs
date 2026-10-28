use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const GCP_TOKEN_TTL_SECONDS: u64 = 3300;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Gcp,
    Azure,
    Aws,
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
    pub gcp: Option<GcpSettings>,
    pub azure: Option<AzureSettings>,
    pub aws: Option<AwsSettings>,
    pub url: Option<String>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub warnings: Vec<ProviderWarning>,
    pub environment_snapshot: BTreeMap<String, bool>,
}

#[derive(Debug, Clone)]
pub struct GcpSettings {
    pub service_account: SecretString,
    pub ssl_ca_certs: Option<String>,
    pub token_ttl_seconds: u64,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let settings = self.litellm_settings.cache_params;
        let environment_snapshot = environment_snapshot(lookup);
        let selection = resolve_providers(&settings, lookup)?;
        let provider = provider_precedence(
            selection.gcp.as_ref(),
            selection.azure.as_ref(),
            selection.aws.as_ref(),
        );
        let (url, username, password) = resolve_connection(&settings, lookup, provider.is_some());

        Ok(Settings {
            provider,
            gcp: selection.gcp,
            azure: selection.azure,
            aws: selection.aws,
            url,
            username,
            password,
            warnings: selection.warnings,
            environment_snapshot,
        })
    }
}

struct ProviderSelection {
    gcp: Option<GcpSettings>,
    azure: Option<AzureSettings>,
    aws: Option<AwsSettings>,
    warnings: Vec<ProviderWarning>,
}

fn resolve_providers<L>(
    settings: &RawSettings,
    lookup: &L,
) -> Result<ProviderSelection, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut warnings = Vec::new();
    let gcp = build_gcp(settings, lookup);
    let azure = resolve_azure_provider(settings, lookup, gcp.as_ref(), &mut warnings);
    let aws = resolve_aws_provider(
        settings,
        lookup,
        gcp.as_ref(),
        azure.as_ref(),
        &mut warnings,
    )?;

    Ok(ProviderSelection {
        gcp,
        azure,
        aws,
        warnings,
    })
}

fn build_gcp<L>(settings: &RawSettings, lookup: &L) -> Option<GcpSettings>
where
    L: EnvironmentLookup,
{
    let gcp_service_account = settings
        .gcp_service_account
        .clone()
        .or_else(|| lookup.lookup("REDIS_GCP_SERVICE_ACCOUNT"));
    let gcp_ssl_ca_certs = settings
        .gcp_ssl_ca_certs
        .clone()
        .or_else(|| lookup.lookup("REDIS_GCP_SSL_CA_CERTS"));
    let ssl = ssl_enabled(settings, lookup);
    gcp_service_account.map(|account| GcpSettings {
        service_account: SecretString::from(account),
        ssl_ca_certs: gcp_ssl_ca_certs.filter(|_| ssl),
        token_ttl_seconds: GCP_TOKEN_TTL_SECONDS,
    })
}

fn resolve_azure_provider<L>(
    settings: &RawSettings,
    lookup: &L,
    gcp: Option<&GcpSettings>,
    warnings: &mut Vec<ProviderWarning>,
) -> Option<AzureSettings>
where
    L: EnvironmentLookup,
{
    let azure_ad_enabled = settings
        .azure_redis_ad_token
        .clone()
        .or_else(|| lookup.lookup("REDIS_AZURE_AD_TOKEN"))
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    if azure_ad_enabled && gcp.is_some() {
        warnings.push(ProviderWarning::GcpOutranksAzure);
    }
    (azure_ad_enabled && gcp.is_none()).then(|| resolve_azure(settings, lookup))
}

fn resolve_aws_provider<L>(
    settings: &RawSettings,
    lookup: &L,
    gcp: Option<&GcpSettings>,
    azure: Option<&AzureSettings>,
    warnings: &mut Vec<ProviderWarning>,
) -> Result<Option<AwsSettings>, ConfigError>
where
    L: EnvironmentLookup,
{
    let aws_iam_enabled = settings.aws_iam_auth.as_ref().is_some_and(coerces_to_true)
        || lookup
            .lookup("REDIS_AWS_IAM_AUTH")
            .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)));
    if aws_iam_enabled && gcp.is_some() {
        warnings.push(ProviderWarning::GcpOutranksAws);
    } else if aws_iam_enabled && azure.is_some() {
        warnings.push(ProviderWarning::AzureOutranksAws);
    } else {
        // neither conflict applies; the branch below may still build the provider
    }
    (aws_iam_enabled && gcp.is_none() && azure.is_none())
        .then(|| build_aws(settings, lookup))
        .transpose()
}

fn resolve_connection<L>(
    settings: &RawSettings,
    lookup: &L,
    provider_active: bool,
) -> (Option<String>, Option<String>, Option<SecretString>)
where
    L: EnvironmentLookup,
{
    let configured_url = settings.url.clone().or_else(|| lookup.lookup("REDIS_URL"));
    let configured_username = settings
        .username
        .clone()
        .or_else(|| lookup.lookup("REDIS_USERNAME"));
    let configured_password = settings
        .password
        .as_ref()
        .map(|value| value.expose_secret().to_owned())
        .or_else(|| lookup.lookup("REDIS_PASSWORD"));
    if provider_active {
        (
            configured_url.map(|value| url_without_userinfo(&value)),
            None,
            None,
        )
    } else {
        (
            configured_url,
            configured_username,
            configured_password.map(SecretString::from),
        )
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
    let fallback = || {
        lookup
            .lookup("REDIS_SSL")
            .is_some_and(|value| coerces_to_true(&ScalarBool::Text(value)))
    };
    settings.ssl.as_ref().map_or_else(fallback, coerces_to_true)
}

const fn provider_precedence(
    gcp: Option<&GcpSettings>,
    azure: Option<&AzureSettings>,
    aws: Option<&AwsSettings>,
) -> Option<Provider> {
    if gcp.is_some() {
        Some(Provider::Gcp)
    } else if azure.is_some() {
        Some(Provider::Azure)
    } else if aws.is_some() {
        Some(Provider::Aws)
    } else {
        None
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

fn url_without_userinfo(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    let _ignored_username = parsed.set_username("");
    let _ignored_password = parsed.set_password(None);
    parsed.to_string()
}

#[path = "redis_credentials/aws.rs"]
pub mod aws;
use aws::{AwsSettings, build_aws};

#[path = "redis_credentials/azure.rs"]
pub mod azure;
use azure::{AzureSettings, resolve_azure};

#[path = "redis_credentials/tests.rs"]
mod tests;
