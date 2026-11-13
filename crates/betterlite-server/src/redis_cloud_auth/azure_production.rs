use std::sync::{Arc, Mutex};

use betterlite_azure::azure_arc_identity::ArcTransport;
use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::StdFiles;
use betterlite_azure::azure_client_secret::provider::ProviderCache;
use betterlite_azure::azure_client_secret::transport::Clock as TokenClock;
use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_default_chain::grant::{
    ChainPlacement, ChainToken, ClientSecretRecord, Grant as _, Grants as _, LinkError,
    ManagedIdentityRecord,
};
use betterlite_azure::azure_default_chain::native::NativeGrants;
use betterlite_azure::azure_default_chain::provider::{
    ProviderRequest, get_azure_ad_token_provider,
};
use betterlite_azure::azure_hosted_identity::request::{HostedIo, HostedTransport};
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::imds::ImdsTransport;
use betterlite_azure::azure_managed_identity::runtime::{FsTokenFiles, ImdsIo};
use betterlite_azure::azure_username_password::transport::Transport as PasswordTransport;
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_config::sections::azure_client_secret::Settings as ClientSecretSettings;
use betterlite_config::sections::azure_default_chain::AzureCredentialType;
use betterlite_redact::secret::Redactor;
use secrecy::SecretString;

use super::azure::{AzureGrantError, AzureGrants};
use super::azure_http::AzureIo;
use super::azure_runtime::{ProductionClock, ProductionHost, ThreadScheduler};

pub struct CachedToken {
    pub token: SecretString,
    pub expires_at: i64,
}

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
    pub client_secret_settings: ClientSecretSettings,
    pub client_secret_token: Mutex<Option<CachedToken>>,
    pub managed_identity_token: Mutex<Option<CachedToken>>,
    pub default_token: Mutex<Option<CachedToken>>,
}

impl<H: AzureIo> SdkAzureGrants<H> {
    fn now(&self) -> i64 {
        TokenClock::now_unix_seconds(&*self.clock)
    }

    fn native_grants(&self) -> NativeGrants<'_, ProductionHost> {
        let hosted: &dyn HostedTransport = &*self.http;
        let imds: &dyn ImdsTransport = &*self.http;
        let arc: &dyn ArcTransport = &*self.http;
        let password: &dyn PasswordTransport = &*self.http;
        NativeGrants {
            transport: Arc::clone(&self.http),
            clock: Arc::clone(&self.clock),
            client_secrets: Mutex::new(ProviderCache::new(
                Arc::clone(&self.http),
                Arc::clone(&self.clock),
                self.client_secret_settings.clone(),
            )),
            files: &self.files,
            ids: Arc::clone(&self.ids),
            token_files: &self.token_files,
            exchange: &self.exchange,
            managed_environment: &self.environment,
            host: &self.host,
            hosted: HostedIo {
                transport: hosted,
                clock: &*self.clock,
                redactor: &self.redactor,
            },
            arc,
            imds: ImdsIo {
                transport: imds,
                scheduler: &self.scheduler,
                ids: &*self.ids,
                clock: &*self.clock,
            },
            password_transport: password,
            authority_host: self.authority_host.clone(),
        }
    }
}

fn cached(
    cell: &Mutex<Option<CachedToken>>,
    now: i64,
) -> Result<Option<SecretString>, AzureGrantError> {
    let guard = cell.lock().map_err(|_error| AzureGrantError::Grant {
        kind: "token cache",
        message: "the token cache is poisoned".to_owned(),
    })?;
    Ok(guard
        .as_ref()
        .filter(|cached| now < cached.expires_at)
        .map(|cached| cached.token.clone()))
}

fn remember(cell: &Mutex<Option<CachedToken>>, token: &ChainToken) -> Result<(), AzureGrantError> {
    let Some(expires_at) = token.refresh_on.or(token.expires_on) else {
        return Ok(());
    };
    let mut guard = cell.lock().map_err(|_error| AzureGrantError::Grant {
        kind: "token cache",
        message: "the token cache is poisoned".to_owned(),
    })?;
    *guard = Some(CachedToken {
        token: token.token.clone(),
        expires_at,
    });
    Ok(())
}

fn grant_error(kind: &'static str, error: &LinkError) -> AzureGrantError {
    let message = match error {
        LinkError::Unavailable(message) | LinkError::Authentication(message) => message.clone(),
    };
    AzureGrantError::Grant { kind, message }
}

impl<H: AzureIo> AzureGrants for SdkAzureGrants<H> {
    fn client_secret(
        &self,
        tenant_id: &str,
        client_id: &str,
        client_secret: &SecretString,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let now = self.now();
        if let Some(token) = cached(&self.client_secret_token, now)? {
            return Ok(token);
        }
        let record = ClientSecretRecord {
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.clone(),
            scope: scope.to_owned(),
        };
        let native = self.native_grants();
        let grant = native
            .client_secret(&record)
            .map_err(|error| AzureGrantError::Grant {
                kind: "client secret",
                message: error.message,
            })?;
        let token = grant
            .token()
            .map_err(|error| grant_error("client secret", &error))?;
        remember(&self.client_secret_token, &token)?;
        Ok(token.token)
    }

    fn managed_identity(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let now = self.now();
        if let Some(token) = cached(&self.managed_identity_token, now)? {
            return Ok(token);
        }
        let record = ManagedIdentityRecord {
            client_id: Some(client_id.to_owned()),
            exclude_workload_identity: false,
            imds_probe: None,
            placement: ChainPlacement::Standalone,
            scope: scope.to_owned(),
        };
        let native = self.native_grants();
        let grant = native
            .managed_identity(&record)
            .map_err(|error| AzureGrantError::Grant {
                kind: "managed identity",
                message: error.message,
            })?;
        let token = grant
            .token()
            .map_err(|error| grant_error("managed identity", &error))?;
        remember(&self.managed_identity_token, &token)?;
        Ok(token.token)
    }

    fn default_credential(&self, scope: &str) -> Result<SecretString, AzureGrantError> {
        let now = self.now();
        if let Some(token) = cached(&self.default_token, now)? {
            return Ok(token);
        }
        let native = self.native_grants();
        let provider = get_azure_ad_token_provider(&ProviderRequest {
            scope: Some(scope),
            credential: Some(AzureCredentialType::DefaultAzureCredential),
            environment: &self.chain_environment,
            grants: &native,
        })
        .map_err(|source| AzureGrantError::DefaultCredential { source })?;
        let token = provider
            .token()
            .map_err(|source| AzureGrantError::DefaultCredential { source })?;
        remember(&self.default_token, &token)?;
        Ok(token.token)
    }
}
