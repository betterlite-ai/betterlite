use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_certificate::assertion::RandomIds;
use betterlite_azure::azure_certificate::credential::StdFiles;
use betterlite_azure::azure_client_secret::provider::ProviderCache;
use betterlite_azure::azure_client_secret::transport::Clock as TokenClock;
use betterlite_azure::azure_default_chain::environment::ChainEnvironment;
use betterlite_azure::azure_default_chain::grant::{
    ChainPlacement, ChainToken, ClientSecretRecord, Grant, GrantError, Grants as _, LinkError,
    ManagedIdentityRecord,
};
use betterlite_azure::azure_default_chain::native::NativeGrants;
use betterlite_azure::azure_default_chain::provider::{
    ProviderRequest, get_azure_ad_token_provider,
};
use betterlite_azure::azure_hosted_identity::request::HostedIo;
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::runtime::{FsTokenFiles, ImdsIo};
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_config::sections::azure_client_secret::Settings as ClientSecretSettings;
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
    pub client_secret_settings: ClientSecretSettings,
    pub client_secrets: KeptProvider,
    pub managed_identity_tokens: TokenCache,
    pub default_tokens: TokenCache,
}

struct KeptToken {
    key: Vec<String>,
    token: ChainToken,
}

// betterlite-azure caches only the client secret grant, so managed identity and the default chain keep their token here
#[derive(Default)]
pub struct TokenCache {
    kept: Mutex<Option<KeptToken>>,
}

impl TokenCache {
    // the lock stays held through the fetch, so concurrent callers wait and then read the new token
    pub fn get_or_fetch<F>(
        &self,
        key: &[&str],
        clock: &dyn TokenClock,
        fetch: F,
    ) -> Result<ChainToken, AzureGrantError>
    where
        F: FnOnce() -> Result<ChainToken, AzureGrantError>,
    {
        let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        let now = clock.now_unix_seconds();
        let cached = kept
            .as_ref()
            .filter(|slot| slot.key.iter().map(String::as_str).eq(key.iter().copied()))
            .map(|slot| slot.token.clone());
        if let Some(token) = &cached
            && let Some(expires_on) = token.expires_on
            && now < token.refresh_on.unwrap_or(expires_on)
        {
            return Ok(token.clone());
        }
        match fetch() {
            Ok(token) => {
                *kept = Some(KeptToken {
                    key: key.iter().copied().map(str::to_owned).collect(),
                    token: token.clone(),
                });
                drop(kept);
                Ok(token)
            }
            // azure-identity serves the aging token when its refresh fails
            Err(error) => match cached {
                Some(token) if token.expires_on.is_some_and(|expires_on| now < expires_on) => {
                    tracing::warn!(
                        "refreshing the Azure AD token failed, serving the cached one: {error}"
                    );
                    Ok(token)
                }
                _ => Err(error),
            },
        }
    }
}

// NativeGrants owns its provider cache, so the kept one is moved in for a fetch and back out after it
#[derive(Default)]
pub struct KeptProvider {
    slot: Mutex<Option<ProviderCache>>,
}

impl KeptProvider {
    // the guard stays held for the whole fetch, so concurrent fetches queue behind one provider cache
    fn lend<C, W, T>(&self, create: C, work: W) -> T
    where
        C: FnOnce() -> ProviderCache,
        W: FnOnce(ProviderCache) -> (Mutex<ProviderCache>, T),
    {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        let cache = slot.take().unwrap_or_else(create);
        let (returned, result) = work(cache);
        *slot = Some(
            returned
                .into_inner()
                .unwrap_or_else(PoisonError::into_inner),
        );
        drop(slot);
        result
    }
}

impl<H: AzureIo> SdkAzureGrants<H> {
    fn token_through<F>(&self, fetch: F) -> Result<ChainToken, AzureGrantError>
    where
        F: FnOnce(&NativeGrants<'_, ProductionHost>) -> Result<ChainToken, AzureGrantError>,
    {
        self.client_secrets.lend(
            || {
                ProviderCache::new(
                    Arc::<H>::clone(&self.http),
                    Arc::<ProductionClock>::clone(&self.clock),
                    self.client_secret_settings.clone(),
                )
            },
            |cache| {
                let native = NativeGrants {
                    transport: Arc::<H>::clone(&self.http),
                    clock: Arc::<ProductionClock>::clone(&self.clock),
                    client_secrets: Mutex::new(cache),
                    files: &self.files,
                    ids: Arc::<RandomIds>::clone(&self.ids),
                    token_files: &self.token_files,
                    exchange: &self.exchange,
                    managed_environment: &self.environment,
                    host: &self.host,
                    hosted: HostedIo {
                        transport: &*self.http,
                        clock: &*self.clock,
                        redactor: &self.redactor,
                    },
                    arc: &*self.http,
                    imds: ImdsIo {
                        transport: &*self.http,
                        scheduler: &self.scheduler,
                        ids: &*self.ids,
                        clock: &*self.clock,
                    },
                    password_transport: &*self.http,
                    authority_host: self.authority_host.clone(),
                };
                let result = fetch(&native);
                (native.client_secrets, result)
            },
        )
    }
}

fn grant_token<G>(
    grant: Result<G, GrantError>,
    kind: &'static str,
) -> Result<ChainToken, AzureGrantError>
where
    G: Grant,
{
    grant
        .map_err(|error| AzureGrantError::Grant {
            kind,
            message: error.message,
        })?
        .token()
        .map_err(|error| {
            let (LinkError::Unavailable(message) | LinkError::Authentication(message)) = error;
            AzureGrantError::Grant { kind, message }
        })
}

impl<H: AzureIo> AzureGrants for SdkAzureGrants<H> {
    fn client_secret(
        &self,
        tenant_id: &str,
        client_id: &str,
        client_secret: &SecretString,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let record = ClientSecretRecord {
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            client_secret: client_secret.clone(),
            scope: scope.to_owned(),
        };
        self.token_through(|native| grant_token(native.client_secret(&record), "client secret"))
            .map(|token| token.token)
    }

    fn managed_identity(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let record = ManagedIdentityRecord {
            client_id: Some(client_id.to_owned()),
            exclude_workload_identity: false,
            imds_probe: None,
            placement: ChainPlacement::Standalone,
            scope: scope.to_owned(),
        };
        let fetched =
            self.managed_identity_tokens
                .get_or_fetch(&[client_id, scope], &*self.clock, || {
                    self.token_through(|native| {
                        grant_token(native.managed_identity(&record), "managed identity")
                    })
                })?;
        Ok(fetched.token)
    }

    fn default_credential(&self, scope: &str) -> Result<SecretString, AzureGrantError> {
        self.default_tokens
            .get_or_fetch(&[scope], &*self.clock, || {
                self.token_through(|native| {
                    let provider = get_azure_ad_token_provider(&ProviderRequest {
                        scope: Some(scope),
                        credential: Some(AzureCredentialType::DefaultAzureCredential),
                        environment: &self.chain_environment,
                        grants: native,
                    })
                    .map_err(|source| AzureGrantError::DefaultCredential { source })?;
                    provider
                        .token()
                        .map_err(|source| AzureGrantError::DefaultCredential { source })
                })
            })
            .map(|token| token.token)
    }
}
