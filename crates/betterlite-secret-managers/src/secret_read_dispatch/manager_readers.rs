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
        let target = self.manager.build_secret_target(secret_name, None).ok()?;
        let token = self.manager.request_token_blocking().ok()?;
        let response = self
            .transport
            .send_blocking(VaultRequest {
                method: VaultMethod::Get,
                url: target.url,
                token: Some(token),
                namespace: self.manager.vault_secret_namespace().map(str::to_owned),
                body: None,
                client_certificate: self.certificate(),
                timeout: READ_TIMEOUT,
                follow_redirects: true,
                max_response_bytes: MAX_RESPONSE_BYTES,
            })
            .ok()?;
        if !(200..300).contains(&response.status) {
            return None;
        }
        secret_value(response.body.expose_secret().as_bytes(), &target.data_key)
    }
}

impl<T: ConjurTransport> SyncSecretReader for CyberArkSecretManager<T> {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        <Self as BaseSecretManager>::sync_read_secret(self, secret_name, SecretOptions::default())
            .ok()
            .flatten()
    }
}
