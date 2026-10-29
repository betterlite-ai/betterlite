use std::collections::BTreeMap;
use std::time::SystemTime;

use aws_credential_types::Credentials;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::redis_credentials::Provider;
use betterlite_config::sections::redis_credentials::aws::{
    AwsCredentialsResolver, ElastiCacheIamProvider, ElastiCacheSigningError,
};
use betterlite_config::sections::redis_credentials::azure::AzureCredential;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GcpTokenCaching {
        service_account: String,
    },
    ElasticacheSigGeneration {
        cache_name: String,
        #[serde(default)]
        serverless: bool,
    },
    AzureCredentialSelection {
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default)]
        tenant_id: Option<String>,
        #[serde(default)]
        client_secret: Option<String>,
    },
}

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("expected a {expected} provider, resolved {resolved}")]
    WrongProvider {
        expected: &'static str,
        resolved: &'static str,
    },
    #[error(transparent)]
    Signing(#[from] ElastiCacheSigningError),
}

struct StaticAwsCredentials;

impl AwsCredentialsResolver for StaticAwsCredentials {
    fn resolve(&self) -> Result<Credentials, ElastiCacheSigningError> {
        Ok(Credentials::new(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            None,
            None,
            "redis-credentials-helper",
        ))
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::GcpTokenCaching { service_account } => gcp_token_caching(service_account),
            Input::ElasticacheSigGeneration {
                cache_name,
                serverless,
            } => elasticache_sig_generation(cache_name, *serverless),
            Input::AzureCredentialSelection {
                client_id,
                tenant_id,
                client_secret,
            } => azure_credential_selection(
                client_id.as_deref(),
                tenant_id.as_deref(),
                client_secret.as_deref(),
            ),
        };
        Ok(output?)
    }
}

fn load(cache_params: &Value) -> Result<betterlite_config::types::Config, ConfigError> {
    let config = json!({
        "litellm_settings": {
            "cache_params": cache_params,
        }
    })
    .to_string();
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(&config)
}

const fn provider_name(provider: &Provider) -> &'static str {
    match provider {
        Provider::Gcp(_) => "gcp",
        Provider::Azure(_) => "azure",
        Provider::Aws(_) => "aws",
    }
}

fn gcp_token_caching(service_account: &str) -> Result<Value, Failure> {
    if service_account.is_empty() {
        return Ok(json!({
            "provider": "none",
            "service_account_configured": false,
            "token_ttl_seconds": null,
        }));
    }
    let loaded = load(&json!({ "gcp_service_account": service_account }))?;
    match loaded.sections.redis_credentials.provider {
        Some(Provider::Gcp(gcp)) => Ok(json!({
            "provider": "gcp",
            "service_account_configured": !gcp.service_account.expose_secret().is_empty(),
            "token_ttl_seconds": gcp.token_ttl_seconds,
        })),
        other => Err(Failure::WrongProvider {
            expected: "gcp",
            resolved: other.as_ref().map_or("none", provider_name),
        }),
    }
}

fn elasticache_sig_generation(cache_name: &str, serverless: bool) -> Result<Value, Failure> {
    let loaded = load(&json!({
        "aws_iam_auth": true,
        "aws_iam_user_name": "connect",
        "aws_iam_cache_name": cache_name,
        "aws_iam_region": "us-east-1",
        "aws_iam_serverless": serverless,
        "ssl": true,
    }))?;
    match loaded.sections.redis_credentials.provider {
        Some(Provider::Aws(aws)) => {
            let provider = ElastiCacheIamProvider { settings: aws };
            let (_user_name, signed) =
                provider.get_credentials(&StaticAwsCredentials, SystemTime::UNIX_EPOCH)?;
            let signed_url = signed.expose_secret();
            let (host, query) = signed_url.split_once("/?").unwrap_or((signed_url, ""));
            let params: BTreeMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
                .map(|(name, value)| (name.into_owned(), value.into_owned()))
                .collect();
            let mut result = serde_json::Map::new();
            result.insert("provider".to_owned(), json!("aws"));
            result.insert("host".to_owned(), json!(host));
            result.insert("User".to_owned(), json!(params.get("User")));
            result.insert("Action".to_owned(), json!(params.get("Action")));
            if let Some(resource_type) = params.get("ResourceType") {
                result.insert("ResourceType".to_owned(), json!(resource_type));
            }
            Ok(Value::Object(result))
        }
        other => Err(Failure::WrongProvider {
            expected: "aws",
            resolved: other.as_ref().map_or("none", provider_name),
        }),
    }
}

fn azure_credential_selection(
    client_id: Option<&str>,
    tenant_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<Value, Failure> {
    let mut cache_params = serde_json::Map::new();
    cache_params.insert("azure_redis_ad_token".to_owned(), json!(true));
    if let Some(value) = client_id {
        cache_params.insert("azure_client_id".to_owned(), json!(value));
    }
    if let Some(value) = tenant_id {
        cache_params.insert("azure_tenant_id".to_owned(), json!(value));
    }
    if let Some(value) = client_secret {
        cache_params.insert("azure_client_secret".to_owned(), json!(value));
    }
    let loaded = load(&Value::Object(cache_params))?;
    match loaded.sections.redis_credentials.provider {
        Some(Provider::Azure(azure)) => Ok(json!({
            "provider": "azure",
            "credential": match azure.credential {
                AzureCredential::ClientSecret => "client_secret",
                AzureCredential::ManagedIdentity => "managed_identity",
                AzureCredential::Default => "default",
            },
        })),
        other => Err(Failure::WrongProvider {
            expected: "azure",
            resolved: other.as_ref().map_or("none", provider_name),
        }),
    }
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::Config(_) => "ConfigError",
            Failure::WrongProvider { .. } => "AssertionError",
            Failure::Signing(_) => "RuntimeError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}
