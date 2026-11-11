use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::SecretString;

use crate::azure_arc_identity::{ArcHost, ArcTransport};
use crate::azure_certificate::assertion::IdSource;
use crate::azure_certificate::credential::{CertificateCredential, CertificateInput, FileReader};
use crate::azure_client_secret::provider::{ClientSecretProvider, ProviderCache};
use crate::azure_client_secret::transport::{Clock, Transport};
use crate::azure_client_secret::{ClientSecretInput, SecretResolver};
use crate::azure_hosted_identity::request::HostedIo;
use crate::azure_managed_identity::environment::ManagedIdentityEnvironment;
use crate::azure_managed_identity::runtime::{ImdsIo, ManagedIdentityIo, TokenFiles, WorkloadIo};
use crate::azure_managed_identity::selection::WorkloadSelection;
use crate::azure_username_password::UsernamePasswordInput;
use crate::azure_username_password::transport::Transport as PasswordTransport;
use crate::oidc_exchange::AzureOidcExchange;

use super::grant::{
    CertificateRecord, ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};

pub mod managed;
pub mod password;
pub mod workload;

const DEFAULT_AUTHORITY_HOST: &str = "login.microsoftonline.com";

// every value was resolved from the environment before a record was built
pub struct ResolvedValues;

impl SecretResolver for ResolvedValues {
    fn get_secret_str(&self, _reference: &str) -> Option<SecretString> {
        None
    }
}

pub struct NativeGrants<'io, H: ArcHost> {
    pub transport: Arc<dyn Transport>,
    pub clock: Arc<dyn Clock>,
    pub client_secrets: Mutex<ProviderCache>,
    pub files: &'io dyn FileReader,
    pub ids: Arc<dyn IdSource>,
    pub token_files: &'io dyn TokenFiles,
    pub exchange: &'io AzureOidcExchange,
    pub managed_environment: &'io ManagedIdentityEnvironment,
    pub host: &'io H,
    pub hosted: HostedIo<'io>,
    pub arc: &'io dyn ArcTransport,
    pub imds: ImdsIo<'io>,
    pub password_transport: &'io dyn PasswordTransport,
    pub authority_host: Option<String>,
}

pub enum NativeGrant<'io, H: ArcHost> {
    ClientSecret(Arc<ClientSecretProvider>),
    Certificate(CertificateCredential),
    UsernamePassword(password::EnvironmentPasswordGrant<'io>),
    Workload(workload::WorkloadGrant<'io>),
    ManagedIdentity(managed::ManagedIdentityGrant<'io, H>),
}

impl<H: ArcHost> Grant for NativeGrant<'_, H> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        match self {
            Self::ClientSecret(provider) => {
                let token = provider
                    .token()
                    .map_err(|error| LinkError::Authentication(error.to_string()))?;
                Ok(ChainToken {
                    token: token.token,
                    expires_on: Some(token.expires_on),
                    token_type: Some(token.token_type),
                    refresh_on: token.refresh_on,
                })
            }
            Self::Certificate(credential) => {
                let token = credential
                    .token()
                    .map_err(|error| LinkError::Authentication(error.to_string()))?;
                Ok(ChainToken {
                    token: token.token,
                    expires_on: Some(token.expires_on),
                    token_type: Some(token.token_type),
                    refresh_on: token.refresh_on,
                })
            }
            Self::UsernamePassword(grant) => grant.token(),
            Self::Workload(grant) => grant.token(),
            Self::ManagedIdentity(grant) => grant.token(),
        }
    }
}

fn poison_error(credential: &'static str) -> GrantError {
    GrantError {
        credential,
        message: "the provider cache is poisoned".to_owned(),
    }
}

impl<H: ArcHost> Grants for NativeGrants<'_, H> {
    type Grant<'grants>
        = NativeGrant<'grants, H>
    where
        Self: 'grants;

    fn client_secret<'grants>(
        &'grants self,
        record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        let input = ClientSecretInput {
            tenant_id: record.tenant_id.clone(),
            client_id: record.client_id.clone(),
            client_secret: record.client_secret.clone(),
            scope: Some(record.scope.clone()),
        };
        let mut cache: MutexGuard<'_, ProviderCache> = self
            .client_secrets
            .lock()
            .map_err(|_error| poison_error("ClientSecretCredential"))?;
        let provider = cache
            .provider(&input, &ResolvedValues, self.authority_host.as_deref())
            .map_err(|error| GrantError {
                credential: "ClientSecretCredential",
                message: error.to_string(),
            })?;
        drop(cache);
        Ok(NativeGrant::ClientSecret(provider))
    }

    fn certificate<'grants>(
        &'grants self,
        record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        let input = CertificateInput {
            tenant_id: record.tenant_id.clone(),
            client_id: record.client_id.clone(),
            scope: Some(record.scope.clone()),
            certificate_path: Some(record.certificate_path.clone()),
            certificate_data: None,
            password: record.password.clone(),
            send_certificate_chain: record.send_certificate_chain,
            authority_host: self.authority_host.clone(),
        };
        let credential = CertificateCredential::new(
            input,
            self.files,
            Arc::clone(&self.transport),
            Arc::clone(&self.clock),
            Arc::clone(&self.ids),
        )
        .map_err(|error| GrantError {
            credential: "CertificateCredential",
            message: error.to_string(),
        })?;
        Ok(NativeGrant::Certificate(credential))
    }

    fn username_password<'grants>(
        &'grants self,
        record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(NativeGrant::UsernamePassword(
            password::EnvironmentPasswordGrant {
                input: UsernamePasswordInput {
                    client_id: record.client_id.clone(),
                    username: record.username.clone(),
                    password: record.password.clone(),
                    scope: Some(record.scope.clone()),
                },
                tenant: record
                    .tenant_id
                    .as_deref()
                    .filter(|tenant| !tenant.is_empty())
                    .unwrap_or("organizations")
                    .to_owned(),
                authority_host: self
                    .authority_host
                    .clone()
                    .unwrap_or_else(|| DEFAULT_AUTHORITY_HOST.to_owned()),
                transport: self.password_transport,
                clock: Arc::clone(&self.clock),
                state: Mutex::new(password::State::default()),
            },
        ))
    }

    fn workload<'grants>(
        &'grants self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(NativeGrant::Workload(workload::WorkloadGrant {
            exchange: self.exchange,
            files: self.token_files,
            selection: WorkloadSelection {
                client_id: record.client_id.clone(),
                tenant_id: record.tenant_id.clone(),
                token_file: record.token_file.clone(),
            },
            scope: record.scope.clone(),
        }))
    }

    fn managed_identity<'grants>(
        &'grants self,
        record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(NativeGrant::ManagedIdentity(
            managed::ManagedIdentityGrant {
                environment: self.managed_environment,
                host: self.host,
                io: ManagedIdentityIo {
                    imds: self.imds,
                    hosted: self.hosted,
                    arc: self.arc,
                    workload: WorkloadIo {
                        exchange: self.exchange,
                        files: self.token_files,
                    },
                },
                client_id: record.client_id.clone(),
                exclude_workload_identity: record.exclude_workload_identity,
                imds_probe: record.imds_probe,
                placement: record.placement,
                scope: record.scope.clone(),
            },
        ))
    }
}
