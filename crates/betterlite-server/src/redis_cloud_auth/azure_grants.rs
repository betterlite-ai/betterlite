use std::sync::{Arc, Mutex};

use betterlite_azure::azure_arc_identity::ArcTransport;
use betterlite_azure::azure_certificate::credential::{CertificateCredential, CertificateInput};
use betterlite_azure::azure_client_secret::transport::Clock as TokenClock;
use betterlite_azure::azure_client_secret::transport::Transport as ClientSecretTransport;
use betterlite_azure::azure_client_secret::{ClientSecretInput, DEFAULT_AUTHORITY_HOST};
use betterlite_azure::azure_default_chain::grant::{
    CertificateRecord, ClientSecretRecord, GrantError, Grants, ManagedIdentityRecord,
    UsernamePasswordRecord, WorkloadRecord,
};
use betterlite_azure::azure_default_chain::native::NativeGrant;
use betterlite_azure::azure_default_chain::native::ResolvedValues;
use betterlite_azure::azure_default_chain::native::managed::ManagedIdentityGrant;
use betterlite_azure::azure_default_chain::native::password::{EnvironmentPasswordGrant, State};
use betterlite_azure::azure_default_chain::native::workload::WorkloadGrant;
use betterlite_azure::azure_hosted_identity::request::{HostedIo, HostedTransport};
use betterlite_azure::azure_managed_identity::imds::ImdsTransport;
use betterlite_azure::azure_managed_identity::runtime::{
    ImdsIo, ManagedIdentityIo, TokenFiles, WorkloadIo,
};
use betterlite_azure::azure_managed_identity::selection::WorkloadSelection;
use betterlite_azure::azure_username_password::UsernamePasswordInput;
use betterlite_azure::azure_username_password::transport::Transport as PasswordTransport;

use super::azure_http::AzureIo;
use super::azure_production::SdkAzureGrants;
use super::azure_runtime::ProductionHost;

impl<H: AzureIo> Grants for SdkAzureGrants<H> {
    type Grant<'grants>
        = NativeGrant<'grants, ProductionHost>
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
        let mut cache = self.client_secrets.lock().map_err(|_error| GrantError {
            credential: "ClientSecretCredential",
            message: "the credential cache is poisoned".to_owned(),
        })?;
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
        let transport: Arc<dyn ClientSecretTransport> = Arc::clone(&self.http);
        let clock: Arc<dyn TokenClock> = Arc::clone(&self.clock);
        let credential =
            CertificateCredential::new(input, &self.files, transport, clock, Arc::clone(&self.ids))
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
        let transport: &dyn PasswordTransport = &*self.http;
        let clock: Arc<dyn TokenClock> = Arc::clone(&self.clock);
        Ok(NativeGrant::UsernamePassword(EnvironmentPasswordGrant {
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
            transport,
            clock,
            state: Mutex::new(State::default()),
        }))
    }

    fn workload<'grants>(
        &'grants self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        Ok(NativeGrant::Workload(WorkloadGrant {
            exchange: &self.exchange,
            files: &self.token_files,
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
        let imds: &dyn ImdsTransport = &*self.http;
        let hosted: &dyn HostedTransport = &*self.http;
        let arc: &dyn ArcTransport = &*self.http;
        let files: &dyn TokenFiles = &self.token_files;
        Ok(NativeGrant::ManagedIdentity(ManagedIdentityGrant {
            environment: &self.environment,
            host: &self.host,
            io: ManagedIdentityIo {
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
            },
            client_id: record.client_id.clone(),
            exclude_workload_identity: record.exclude_workload_identity,
            imds_probe: record.imds_probe,
            placement: record.placement,
            scope: record.scope.clone(),
        }))
    }
}
