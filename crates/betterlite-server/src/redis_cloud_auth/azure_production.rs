use std::sync::{Arc, Mutex};

use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::StdFiles;
use betterlite_azure::azure_client_secret::ClientSecretInput;
use betterlite_azure::azure_client_secret::provider::ProviderCache;
use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_default_chain::native::ResolvedValues;
use betterlite_azure::azure_default_chain::provider::{
    ProviderRequest, get_azure_ad_token_provider,
};
use betterlite_azure::azure_hosted_identity::request::HostedIo;
use betterlite_azure::azure_managed_identity::ManagedIdentityCredential;
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::imds::ProbePolicy;
use betterlite_azure::azure_managed_identity::runtime::{
    FsTokenFiles, ImdsIo, ManagedIdentityIo, WorkloadIo,
};
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;

use super::azure::{AzureGrantError, AzureGrants};
use super::azure_http::AzureIo;
use super::azure_runtime::{ProductionClock, ProductionHost, ThreadScheduler};

pub struct SdkAzureGrants<H: AzureIo> {
    pub http: Arc<H>,
    pub clock: Arc<ProductionClock>,
    pub files: StdFiles,
    pub ids: Arc<RandomIds>,
    pub token_files: FsTokenFiles,
    pub scheduler: ThreadScheduler,
    pub host: ProductionHost,
    pub redactor: Redactor,
    pub environment: ManagedIdentityEnvironment,
    pub chain_environment: ChainEnvironment,
    pub exchange: AzureOidcExchange,
    pub authority_host: Option<String>,
    pub client_secrets: Mutex<ProviderCache>,
}

impl<H: AzureIo> AzureGrants for SdkAzureGrants<H> {
    fn client_secret(
        &self,
        tenant_id: &str,
        client_id: &str,
        client_secret: &SecretString,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let input = ClientSecretInput {
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.clone(),
            scope: Some(scope.to_owned()),
        };
        let mut cache = self
            .client_secrets
            .lock()
            .map_err(|_error| AzureGrantError::Build {
                kind: "client secret",
                message: "the credential cache is poisoned".to_owned(),
            })?;
        let provider = cache
            .provider(&input, &ResolvedValues, self.authority_host.as_deref())
            .map_err(|source| AzureGrantError::ClientSecret { source })?;
        drop(cache);
        provider
            .token()
            .map(|token| token.token)
            .map_err(|source| AzureGrantError::ClientSecret { source })
    }

    fn managed_identity(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let credential = ManagedIdentityCredential {
            environment: &self.environment,
            host: &self.host,
            client_id: Some(client_id),
            exclude_workload_identity: false,
            within_default_credential: false,
            probe: ProbePolicy::default(),
        };
        let imds: &dyn betterlite_azure::azure_managed_identity::imds::ImdsTransport = &*self.http;
        let hosted: &dyn betterlite_azure::azure_hosted_identity::request::HostedTransport =
            &*self.http;
        let arc: &dyn betterlite_azure::azure_arc_identity::ArcTransport = &*self.http;
        let files: &dyn betterlite_azure::azure_managed_identity::runtime::TokenFiles =
            &self.token_files;
        let io = ManagedIdentityIo {
            imds: ImdsIo {
                transport: imds,
                scheduler: &self.scheduler,
                ids: &*self.ids,
                clock: &*self.clock,
            },
            hosted: HostedIo {
                transport: hosted,
                clock: &*self.clock,
                redactor: &self.redactor,
            },
            arc,
            workload: WorkloadIo {
                exchange: &self.exchange,
                files,
            },
        };
        credential
            .token(&io, &[scope])
            .map(|token| token.token)
            .map_err(|source| AzureGrantError::ManagedIdentity { source })
    }

    fn default_credential(&self, scope: &str) -> Result<SecretString, AzureGrantError> {
        let provider = get_azure_ad_token_provider(&ProviderRequest {
            scope: Some(scope),
            credential: Some(AzureCredentialType::DefaultAzureCredential),
            environment: &self.chain_environment,
            grants: self,
        })
        .map_err(|source| AzureGrantError::Default { source })?;
        provider
            .token()
            .map(|token| token.token)
            .map_err(|source| AzureGrantError::Default { source })
    }
}
