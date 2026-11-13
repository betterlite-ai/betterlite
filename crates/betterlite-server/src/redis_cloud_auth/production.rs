use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::StdFiles;
use betterlite_azure::azure_client_secret::provider::ProviderCache;
use betterlite_azure::azure_client_secret::transport::{
    Clock as TokenClock, Transport as ClientSecretTransport,
};
use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::runtime::FsTokenFiles;
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_azure::oidc_exchange::transport::{Clock as OidcClock, Transport as OidcTransport};
use betterlite_config::types::Config;
use betterlite_http::proxy::ProxiedClient;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redis::redis_live_client::credentials::RedisCredentialSource;
use betterlite_vertex::credential_sources::files::StdFiles as VertexFiles;
use betterlite_vertex::credential_sources::sdk::{GoogleAuth, HttpPolicy};
use thiserror::Error;
use tokio::runtime::Handle;

use super::aws::load_default_provider;
use super::azure::AzureGrants;
use super::azure_http::AzureHttp;
use super::azure_production::SdkAzureGrants;
use super::azure_runtime::{ProductionClock, ProductionHost, ThreadScheduler};
use super::gcp::AdcTokenSource;
use super::gcp_production::VertexAdcTokenSource;
use super::{CloudRedisAuthDeps, cloud_redis_auth};

#[derive(Debug, Error)]
pub enum CloudRedisAuthSetupError {
    #[error("the Azure OIDC exchange could not be built: {source}")]
    OidcExchange {
        #[source]
        source: betterlite_azure::oidc_exchange::OidcExchangeError,
    },
    #[error("the Azure secret redactor could not be built: {source}")]
    Redactor {
        #[source]
        source: RedactionError,
    },
    #[error("the AWS default credential chain resolved no provider")]
    AwsCredentialsMissing,
}

pub async fn production_cloud_redis_auth(
    config: &Config,
    environment: &BTreeMap<String, String>,
    http: ProxiedClient,
) -> Result<Option<Arc<dyn RedisCredentialSource>>, CloudRedisAuthSetupError> {
    if config.sections.redis_credentials.provider.is_none() {
        return Ok(None);
    }
    let deps = production_deps(config, environment, http).await?;
    Ok(cloud_redis_auth(&config.sections.redis_credentials, &deps))
}

async fn production_deps(
    config: &Config,
    environment: &BTreeMap<String, String>,
    http: ProxiedClient,
) -> Result<CloudRedisAuthDeps, CloudRedisAuthSetupError> {
    let runtime = Handle::current();
    let adc: Arc<dyn AdcTokenSource> = Arc::new(VertexAdcTokenSource {
        runtime: runtime.clone(),
        environment: config.sections.vertex_credential_sources.adc.clone(),
        files: VertexFiles,
        http: GoogleAuth {
            client: http.clone(),
            policy: HttpPolicy::default(),
        },
    });
    let azure = azure_grants(config, environment, &http, &runtime)?;
    let aws = load_default_provider()
        .await
        .ok_or(CloudRedisAuthSetupError::AwsCredentialsMissing)?;
    Ok(CloudRedisAuthDeps {
        runtime,
        http,
        adc,
        azure,
        aws,
    })
}

struct AzureGrantParts {
    http: Arc<AzureHttp>,
    clock: Arc<ProductionClock>,
    exchange: AzureOidcExchange,
    redactor: Redactor,
}

fn azure_grant_parts(
    config: &Config,
    environment: &BTreeMap<String, String>,
    http: &ProxiedClient,
    runtime: &Handle,
) -> Result<AzureGrantParts, CloudRedisAuthSetupError> {
    let owned_http = Arc::new(AzureHttp {
        runtime: runtime.clone(),
        client: http.clone(),
    });
    let clock = Arc::new(ProductionClock);
    let exchange = oidc_exchange(config, environment, &owned_http, &clock)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| CloudRedisAuthSetupError::Redactor { source })?;
    Ok(AzureGrantParts {
        http: owned_http,
        clock,
        exchange,
        redactor,
    })
}

fn azure_grants(
    config: &Config,
    environment: &BTreeMap<String, String>,
    http: &ProxiedClient,
    runtime: &Handle,
) -> Result<Arc<dyn AzureGrants>, CloudRedisAuthSetupError> {
    let parts = azure_grant_parts(config, environment, http, runtime)?;
    let transport: Arc<dyn ClientSecretTransport> = Arc::clone(&parts.http);
    let client_clock: Arc<dyn TokenClock> = Arc::clone(&parts.clock);
    let client_secrets = Mutex::new(ProviderCache::new(
        transport,
        client_clock,
        config.sections.azure_client_secret.clone(),
    ));
    let managed_environment = ManagedIdentityEnvironment::from_lookup(environment);
    let chain_environment =
        ChainEnvironment::from_settings(&config.sections.azure_default_chain, environment);
    let host = ProductionHost {
        environment: environment.clone(),
    };
    Ok(Arc::new(SdkAzureGrants::<AzureHttp> {
        http: parts.http,
        clock: parts.clock,
        files: StdFiles,
        ids: Arc::new(RandomIds),
        token_files: FsTokenFiles,
        scheduler: ThreadScheduler {
            runtime: runtime.clone(),
        },
        host,
        redactor: parts.redactor,
        environment: managed_environment,
        chain_environment,
        exchange: parts.exchange,
        authority_host: environment.get("AZURE_AUTHORITY_HOST").cloned(),
        client_secrets,
    }))
}

fn oidc_exchange(
    config: &Config,
    environment: &BTreeMap<String, String>,
    http: &Arc<AzureHttp>,
    clock: &Arc<ProductionClock>,
) -> Result<AzureOidcExchange, CloudRedisAuthSetupError> {
    let transport: Arc<dyn OidcTransport> = Arc::clone(http);
    let oidc_clock: Arc<dyn OidcClock> = Arc::clone(clock);
    AzureOidcExchange::new(
        config.sections.azure_oidc_exchange.clone(),
        environment.clone(),
        transport,
        oidc_clock,
    )
    .map_err(|source| CloudRedisAuthSetupError::OidcExchange { source })
}
