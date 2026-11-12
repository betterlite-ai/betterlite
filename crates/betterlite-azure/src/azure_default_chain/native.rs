use std::fmt::Display;
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
use crate::azure_managed_identity::runtime::{ImdsIo, TokenFiles};
use crate::azure_managed_identity::selection::WorkloadSelection;
use crate::azure_username_password::transport::Transport as PasswordTransport;
use crate::oidc_exchange::AzureOidcExchange;

use super::grant::{
    CertificateRecord, ChainToken, ClientSecretRecord, Grant, GrantError, Grants, LinkError,
    ManagedIdentityRecord, UsernamePasswordRecord, WorkloadRecord,
};

pub mod managed;
pub mod password;
pub mod workload;

use managed::ManagedIdentityGrant;
use password::EnvironmentPasswordGrant;
use workload::WorkloadGrant;

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
    UsernamePassword(EnvironmentPasswordGrant<'io>),
    Workload(WorkloadGrant<'io>),
    ManagedIdentity(ManagedIdentityGrant<'io, H>),
}

impl<H: ArcHost> Grant for NativeGrant<'_, H> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        match self {
            Self::ClientSecret(provider) => provider
                .token()
                .map(|token| ChainToken {
                    token: token.token,
                    expires_on: Some(token.expires_on),
                    token_type: Some(token.token_type),
                    refresh_on: token.refresh_on,
                })
                .map_err(|error| LinkError::Authentication(error.to_string())),
            Self::Certificate(credential) => credential
                .token()
                .map(|token| ChainToken {
                    token: token.token,
                    expires_on: Some(token.expires_on),
                    token_type: Some(token.token_type),
                    refresh_on: token.refresh_on,
                })
                .map_err(|error| LinkError::Authentication(error.to_string())),
            Self::UsernamePassword(grant) => grant.token(),
            Self::Workload(grant) => grant.token(),
            Self::ManagedIdentity(grant) => grant.token(),
        }
    }
}

fn grant_error<T, E>(credential: &'static str, result: Result<T, E>) -> Result<T, GrantError>
where
    E: Display,
{
    result.map_err(|error| GrantError {
        credential,
        message: error.to_string(),
    })
}

fn poison_error(credential: &'static str) -> GrantError {
    GrantError {
        credential,
        message: "the provider cache is poisoned".to_owned(),
    }
}

fn certificate_input(
    record: &CertificateRecord,
    authority_host: Option<String>,
) -> CertificateInput {
    CertificateInput {
        tenant_id: record.tenant_id.clone(),
        client_id: record.client_id.clone(),
        scope: Some(record.scope.clone()),
        certificate_path: Some(record.certificate_path.clone()),
        certificate_data: None,
        password: record.password.clone(),
        send_certificate_chain: record.send_certificate_chain,
        authority_host,
    }
}

fn certificate_grant<'grants, 'io, H>(
    source: &'grants NativeGrants<'io, H>,
    record: &CertificateRecord,
) -> Result<NativeGrant<'grants, H>, GrantError>
where
    'io: 'grants,
    H: ArcHost,
{
    let credential = CertificateCredential::new(
        certificate_input(record, source.authority_host.clone()),
        source.files,
        Arc::clone(&source.transport),
        Arc::clone(&source.clock),
        Arc::clone(&source.ids),
    );
    let provider = grant_error("CertificateCredential", credential)?;
    Ok(NativeGrant::Certificate(provider))
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
        let provider = grant_error(
            "ClientSecretCredential",
            cache.provider(&input, &ResolvedValues, self.authority_host.as_deref()),
        )?;
        drop(cache);
        Ok(NativeGrant::ClientSecret(provider))
    }

    fn certificate<'grants>(
        &'grants self,
        record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        certificate_grant(self, record)
    }

    fn username_password<'grants>(
        &'grants self,
        record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(NativeGrant::UsernamePassword(
            EnvironmentPasswordGrant::from_record(record, self),
        ))
    }

    fn workload<'grants>(
        &'grants self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(NativeGrant::Workload(WorkloadGrant {
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
        let grant = ManagedIdentityGrant::from_record(record, self);
        Ok(NativeGrant::ManagedIdentity(grant))
    }
}
