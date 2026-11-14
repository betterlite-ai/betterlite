pub mod aws;
pub mod azure;
pub mod azure_http;
pub mod azure_production;
pub mod azure_runtime;
pub mod gcp;
pub mod gcp_production;
pub mod production;

mod tests;

use std::io;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_credential_types::provider::SharedCredentialsProvider;
use betterlite_config::sections::redis_credentials::aws::ElastiCacheIamProvider;
use betterlite_config::sections::redis_credentials::azure::AzureAdCredentialProvider;
use betterlite_config::sections::redis_credentials::gcp::{GcpTokenCache, SystemClock};
use betterlite_config::sections::redis_credentials::{Provider, Settings};
use betterlite_http::proxy::ProxiedClient;
use betterlite_redis::redis_live_client::credentials::{
    RedisCredentialError, RedisCredentialSource, RedisCredentials,
};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::Handle;

use self::aws::SdkAwsCredentials;
use self::azure::{AzureGrants, AzureRedisTokens};
use self::gcp::{AdcTokenSource, IamTokenFetcher};

pub const GCP_REFRESH_AFTER: Duration = Duration::from_mins(55);
pub const AZURE_REFRESH_AFTER: Duration = Duration::from_secs(300);
pub const AWS_REFRESH_AFTER: Duration = Duration::from_mins(14);

pub struct CloudRedisAuthDeps {
    pub runtime: Handle,
    pub http: ProxiedClient,
    pub adc: Arc<dyn AdcTokenSource>,
    pub azure: Arc<dyn AzureGrants>,
    pub aws: SharedCredentialsProvider,
}

enum CloudCredentials {
    Gcp {
        service_account: SecretString,
        fetcher: IamTokenFetcher,
        cache: Arc<GcpTokenCache>,
        ttl: Duration,
    },
    Azure(AzureAdCredentialProvider<AzureRedisTokens>),
    Aws {
        provider: ElastiCacheIamProvider,
        resolver: SdkAwsCredentials,
    },
}

pub struct CloudRedisAuth {
    credentials: CloudCredentials,
    refresh_after: Duration,
}

#[must_use]
pub fn cloud_redis_auth(
    settings: &Settings,
    deps: &CloudRedisAuthDeps,
) -> Option<Arc<dyn RedisCredentialSource>> {
    let provider = settings.provider.as_ref()?;
    let credentials = match provider {
        Provider::Gcp(gcp) => CloudCredentials::Gcp {
            service_account: gcp.service_account.clone(),
            fetcher: IamTokenFetcher {
                runtime: deps.runtime.clone(),
                http: deps.http.clone(),
                base: gcp::IAM_CREDENTIALS_BASE.to_owned(),
                adc: Arc::clone(&deps.adc),
            },
            cache: Arc::clone(&gcp.token_cache),
            ttl: Duration::from_secs(gcp.token_ttl_seconds),
        },
        Provider::Azure(azure) => CloudCredentials::Azure(AzureAdCredentialProvider {
            credential: azure.credential,
            fetcher: Arc::new(AzureRedisTokens {
                settings: azure.clone(),
                grants: Arc::clone(&deps.azure),
            }),
            username: azure.username.clone(),
        }),
        Provider::Aws(aws) => CloudCredentials::Aws {
            provider: ElastiCacheIamProvider {
                settings: aws.clone(),
            },
            resolver: SdkAwsCredentials {
                runtime: deps.runtime.clone(),
                provider: deps.aws.clone(),
            },
        },
    };
    let refresh_after = match &credentials {
        CloudCredentials::Gcp { .. } => GCP_REFRESH_AFTER,
        CloudCredentials::Azure(_) => AZURE_REFRESH_AFTER,
        CloudCredentials::Aws { .. } => AWS_REFRESH_AFTER,
    };
    Some(Arc::new(CloudRedisAuth {
        credentials,
        refresh_after,
    }))
}

impl RedisCredentialSource for CloudRedisAuth {
    fn fetch(&self) -> Result<RedisCredentials, RedisCredentialError> {
        match &self.credentials {
            CloudCredentials::Gcp {
                service_account,
                fetcher,
                cache,
                ttl,
            } => {
                let clock = SystemClock;
                let token = cache
                    .get_or_fetch(service_account.expose_secret(), *ttl, &clock, fetcher)
                    .map_err(|error| RedisCredentialError::Fetch(io::Error::other(error)))?;
                Ok(RedisCredentials {
                    username: None,
                    password: token,
                })
            }
            CloudCredentials::Azure(provider) => {
                let (username, password) = provider
                    .get_credentials()
                    .map_err(|error| RedisCredentialError::Fetch(io::Error::other(error)))?;
                Ok(RedisCredentials { username, password })
            }
            CloudCredentials::Aws { provider, resolver } => {
                let (username, password) = provider
                    .get_credentials(resolver, SystemTime::now())
                    .map_err(|error| RedisCredentialError::Fetch(io::Error::other(error)))?;
                Ok(RedisCredentials {
                    username: Some(username),
                    password,
                })
            }
        }
    }

    fn refresh_after(&self) -> Duration {
        self.refresh_after
    }
}
