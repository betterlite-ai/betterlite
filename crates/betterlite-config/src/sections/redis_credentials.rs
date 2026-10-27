use std::collections::BTreeMap;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::SecretString;
use serde::Deserialize;

const GCP_TOKEN_TTL_SECONDS: u64 = 3300;
const ELASTICACHE_TOKEN_TTL_SECONDS: u64 = 900;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    gcp_service_account: Option<String>,
    gcp_ssl_ca_certs: Option<String>,
    azure_redis_ad_token: Option<String>,
    azure_client_id: Option<String>,
    azure_tenant_id: Option<String>,
    azure_client_secret: Option<String>,
    aws_iam_auth: Option<ScalarBool>,
    aws_iam_user_name: Option<String>,
    aws_iam_cache_name: Option<String>,
    aws_iam_region: Option<String>,
    aws_iam_serverless: Option<ScalarBool>,
    ssl: Option<ScalarBool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ScalarBool {
    Boolean(bool),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider { Gcp, Azure, Aws }

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub provider: Option<Provider>,
    pub gcp: Option<GcpSettings>,
    pub azure: Option<AzureSettings>,
    pub aws: Option<AwsSettings>,
    pub environment_snapshot: BTreeMap<String, bool>,
}

#[derive(Debug, Clone)]
pub struct GcpSettings {
    pub service_account: SecretString,
    pub ssl_ca_certs: Option<String>,
    pub token_ttl_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct AzureSettings {
    pub token: Option<SecretString>,
    pub client_id: Option<SecretString>,
    pub tenant_id: Option<SecretString>,
    pub client_secret: Option<SecretString>,
    pub credential: AzureCredential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureCredential { Default, ManagedIdentity, ClientSecret }

#[derive(Debug, Clone)]
pub struct AwsSettings {
    pub user_name: String,
    pub cache_name: String,
    pub region: String,
    pub serverless: bool,
    pub token_ttl_seconds: u64,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where L: EnvironmentLookup,
    {
        let settings = merge(self.litellm_settings, self.model_list.into_iter().find_map(|item| {
            let params = item.litellm_params;
            (params.gcp_service_account.is_some() || params.azure_redis_ad_token.is_some() || params.aws_iam_auth.is_some()).then_some(params)
        }));
        let environment_names = ["REDIS_GCP_SERVICE_ACCOUNT", "REDIS_AZURE_AD_TOKEN", "AZURE_CLIENT_ID", "AZURE_TENANT_ID", "AZURE_CLIENT_SECRET", "AWS_REGION", "AWS_DEFAULT_REGION"];
        let environment_snapshot = environment_names.into_iter().map(|name| (name.to_owned(), lookup.lookup(name).is_some())).collect();
        let gcp_service_account = choose(settings.gcp_service_account.clone(), lookup, "REDIS_GCP_SERVICE_ACCOUNT");
        let azure_token = choose(settings.azure_redis_ad_token.clone(), lookup, "REDIS_AZURE_AD_TOKEN");
        let azure_client_id = choose(settings.azure_client_id.clone(), lookup, "AZURE_CLIENT_ID");
        let azure_tenant_id = choose(settings.azure_tenant_id.clone(), lookup, "AZURE_TENANT_ID");
        let azure_client_secret = choose(settings.azure_client_secret.clone(), lookup, "AZURE_CLIENT_SECRET");
        let aws_enabled = settings.aws_iam_auth.as_ref().is_some_and(coerces_to_true);
        let gcp = gcp_service_account.map(|service_account| GcpSettings { service_account: SecretString::new(Box::<str>::from(service_account)), ssl_ca_certs: settings.gcp_ssl_ca_certs.clone(), token_ttl_seconds: GCP_TOKEN_TTL_SECONDS });
        let azure = (azure_token.is_some() || azure_client_id.is_some() || azure_tenant_id.is_some() || azure_client_secret.is_some()).then(|| AzureSettings {
            credential: azure_credential(azure_client_id.as_deref(), azure_tenant_id.as_deref(), azure_client_secret.as_deref()),
            token: azure_token.map(|value| SecretString::new(Box::<str>::from(value))), client_id: azure_client_id.map(|value| SecretString::new(Box::<str>::from(value))), tenant_id: azure_tenant_id.map(|value| SecretString::new(Box::<str>::from(value))), client_secret: azure_client_secret.map(|value| SecretString::new(Box::<str>::from(value))),
        });
        let aws = aws_enabled.then(|| build_aws(&settings, lookup)).transpose()?.flatten();
        let provider = if gcp.is_some() { Some(Provider::Gcp) } else if azure.is_some() { Some(Provider::Azure) } else if aws.is_some() { Some(Provider::Aws) } else { None };
        Ok(Settings { provider, gcp, azure, aws, environment_snapshot })
    }
}

fn merge(global: RawSettings, deployment: Option<RawSettings>) -> RawSettings {
    let Some(deployment_settings) = deployment else { return global };
    RawSettings {
        gcp_service_account: deployment_settings.gcp_service_account.or(global.gcp_service_account), gcp_ssl_ca_certs: deployment_settings.gcp_ssl_ca_certs.or(global.gcp_ssl_ca_certs), azure_redis_ad_token: deployment_settings.azure_redis_ad_token.or(global.azure_redis_ad_token), azure_client_id: deployment_settings.azure_client_id.or(global.azure_client_id), azure_tenant_id: deployment_settings.azure_tenant_id.or(global.azure_tenant_id), azure_client_secret: deployment_settings.azure_client_secret.or(global.azure_client_secret), aws_iam_auth: deployment_settings.aws_iam_auth.or(global.aws_iam_auth), aws_iam_user_name: deployment_settings.aws_iam_user_name.or(global.aws_iam_user_name), aws_iam_cache_name: deployment_settings.aws_iam_cache_name.or(global.aws_iam_cache_name), aws_iam_region: deployment_settings.aws_iam_region.or(global.aws_iam_region), aws_iam_serverless: deployment_settings.aws_iam_serverless.or(global.aws_iam_serverless), ssl: deployment_settings.ssl.or(global.ssl),
    }
}

fn choose(value: Option<String>, lookup: &impl EnvironmentLookup, name: &str) -> Option<String> {
    if value.is_some() {
        value
    } else {
        lookup.lookup(name)
    }
}

fn coerces_to_true(setting: &ScalarBool) -> bool {
    match setting { ScalarBool::Boolean(boolean) => *boolean, ScalarBool::Text(text) => matches!(text.to_ascii_lowercase().as_str(), "true" | "1" | "yes") }
}

const fn azure_credential(client_id: Option<&str>, tenant_id: Option<&str>, secret: Option<&str>) -> AzureCredential {
    if client_id.is_some() && tenant_id.is_some() && secret.is_some() { AzureCredential::ClientSecret } else if client_id.is_some() { AzureCredential::ManagedIdentity } else { AzureCredential::Default }
}

fn build_aws<L>(settings: &RawSettings, lookup: &L) -> Result<Option<AwsSettings>, ConfigError>
where L: EnvironmentLookup,
{
    let region = settings.aws_iam_region.clone().or_else(|| lookup.lookup("AWS_REGION")).or_else(|| lookup.lookup("AWS_DEFAULT_REGION"));
    let missing = [("aws_iam_user_name", settings.aws_iam_user_name.as_ref()), ("aws_iam_cache_name", settings.aws_iam_cache_name.as_ref()), ("aws_iam_region", region.as_ref())];
    if let Some((name, None)) = missing.into_iter().find(|(_, value)| value.is_none()) { return Err(ConfigError::MissingEnvironment { variable: name.to_owned(), path: format!("litellm_settings.{name}") }); }
    if !settings.ssl.as_ref().is_some_and(coerces_to_true) { return Err(ConfigError::InvalidScalar { path: "litellm_settings.ssl".to_owned(), value: false }); }
    Ok(Some(AwsSettings { user_name: settings.aws_iam_user_name.clone().unwrap_or_default(), cache_name: settings.aws_iam_cache_name.clone().unwrap_or_default().to_ascii_lowercase(), region: region.unwrap_or_default(), serverless: settings.aws_iam_serverless.as_ref().is_some_and(coerces_to_true), token_ttl_seconds: ELASTICACHE_TOKEN_TTL_SECONDS }))
}

#[path = "redis_credentials/tests.rs"]
mod tests;
