use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use super::protocols::SyncSecretReader;
use crate::cyberark::transport::{ClientCertificate, ConjurTransport};
use crate::cyberark::{Clock, CyberArkSecretManager};
use crate::secret_contracts::BaseSecretManager;
use crate::secret_contracts::options::SecretOptions;
use crate::vault::HashicorpSecretManager;
use crate::vault::error::VaultInitError;
use crate::vault::response::secret_value;
use crate::vault::settings::{VaultAuth, VaultSettings};
use crate::vault::transport::{
    MAX_RESPONSE_BYTES, VaultMethod, VaultRequest, VaultResponse, VaultTransport,
};

const READ_TIMEOUT: Duration = Duration::from_secs(600);

struct PreparedVaultRead {
    request: VaultRequest,
    data_key: String,
}

pub struct VaultManagerReader<T: VaultTransport> {
    manager: HashicorpSecretManager<Arc<T>>,
    transport: Arc<T>,
    settings: VaultSettings,
}

impl<T: VaultTransport> VaultManagerReader<T> {
    pub fn new(
        environment: &BTreeMap<String, String>,
        transport: Arc<T>,
        clock: Clock,
    ) -> Result<Self, VaultInitError> {
        let settings = VaultSettings::from_environment(environment)?;
        let manager = HashicorpSecretManager::new(environment, Arc::clone(&transport), clock)?;
        Ok(Self {
            manager,
            transport,
            settings,
        })
    }

    fn certificate(&self) -> Option<ClientCertificate> {
        match &self.settings.auth {
            VaultAuth::TlsCert {
                client_certificate, ..
            } => Some(client_certificate.clone()),
            VaultAuth::AppRole { .. } | VaultAuth::Token(_) => None,
        }
    }

    fn prepare(&self, secret_name: &str) -> Option<PreparedVaultRead> {
        let target = match self.manager.build_secret_target(secret_name, None) {
            Ok(target) => target,
            Err(error) => {
                tracing::error!("Error reading secret from Hashicorp Vault: {error}");
                return None;
            }
        };
        let token = match self.manager.request_token_blocking() {
            Ok(token) => token,
            Err(error) => {
                tracing::error!("Error reading secret from Hashicorp Vault: {error}");
                return None;
            }
        };
        // the target URL already carries the namespace as a path segment
        let request = VaultRequest {
            method: VaultMethod::Get,
            url: target.url,
            token: Some(token),
            namespace: None,
            body: None,
            client_certificate: self.certificate(),
            timeout: READ_TIMEOUT,
            follow_redirects: true,
            max_response_bytes: MAX_RESPONSE_BYTES,
        };
        Some(PreparedVaultRead {
            request,
            data_key: target.data_key,
        })
    }

    fn fetch(&self, prepared: PreparedVaultRead) -> Option<SecretString> {
        let PreparedVaultRead { request, data_key } = prepared;
        let response = match self.transport.send_blocking(request) {
            Ok(response) => response,
            Err(error) => {
                tracing::error!("Error reading secret from Hashicorp Vault: {error}");
                return None;
            }
        };
        if !(200..300).contains(&response.status) {
            tracing::error!(
                "Error reading secret from Hashicorp Vault: Vault answered with status {}",
                response.status
            );
            return None;
        }
        secret_value(response.body.expose_secret().as_bytes(), &data_key)
    }
}

// The manager owns the transport it was built with; this clone lets the reader send its own read
impl<T: VaultTransport> VaultTransport for Arc<T> {
    type Error = T::Error;

    fn send(
        &self,
        request: VaultRequest,
    ) -> impl Future<Output = Result<VaultResponse, T::Error>> + Send {
        (**self).send(request)
    }

    fn send_blocking(&self, request: VaultRequest) -> Result<VaultResponse, T::Error> {
        (**self).send_blocking(request)
    }
}

impl<T: VaultTransport> SyncSecretReader for VaultManagerReader<T> {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        let prepared = self.prepare(secret_name)?;
        self.fetch(prepared)
    }
}

impl<T: ConjurTransport> SyncSecretReader for CyberArkSecretManager<T> {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        <Self as BaseSecretManager>::sync_read_secret(self, secret_name, SecretOptions::default())
            .ok()
            .flatten()
    }
}
