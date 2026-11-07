use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::time::Instant;

use crate::cyberark::Clock;
use crate::cyberark::ttl_cache::TtlCache;
use crate::secret_name::{UnsafeSecretNameError, raise_if_unsafe_secret_name};
use error::{LoginMethod, VaultError, VaultInitError};
use settings::{DEFAULT_MOUNT, VaultAuth, VaultSettings};
use target::{VaultSecretParams, VaultSecretSettings, VaultSecretTarget};
use transport::VaultTransport;

mod auth;
pub mod error;
pub mod response;
pub mod settings;
pub mod target;
mod tests;
pub mod transport;

pub struct HashicorpSecretManager<T> {
    settings: VaultSettings,
    transport: T,
    clock: Clock,
    token: Mutex<Option<CachedToken>>,
    #[expect(
        dead_code,
        reason = "the secret body cache is built here for B4417's reads and writes, which this task only creates"
    )]
    body_cache: TtlCache,
}

struct CachedToken {
    token: SecretString,
    expiry: Instant,
}

impl<T: VaultTransport> HashicorpSecretManager<T> {
    pub fn new(
        environment: &BTreeMap<String, String>,
        transport: T,
        clock: Clock,
    ) -> Result<Self, VaultInitError> {
        let settings = VaultSettings::from_environment(environment)?;
        Ok(Self {
            body_cache: TtlCache {
                entries: Mutex::new(BTreeMap::new()),
                ttl: settings.refresh_interval,
                clock: Clock::clone(&clock),
            },
            token: Mutex::new(None),
            settings,
            transport,
            clock,
        })
    }

    pub const fn vault_addr(&self) -> &str {
        let Self { settings, .. } = self;
        settings.addr.as_str()
    }

    pub fn vault_login_namespace(&self) -> Option<&str> {
        if let Some(namespace) = self.settings.login_namespace.as_deref() {
            return Some(namespace);
        }
        self.settings.namespace.as_deref()
    }

    pub fn vault_secret_namespace(&self) -> Option<&str> {
        if let Some(namespace) = self.settings.secret_namespace.as_deref() {
            return Some(namespace);
        }
        self.settings.namespace.as_deref()
    }

    #[expect(
        dead_code,
        reason = "B4417 reads and writes the secret body cache through this accessor"
    )]
    pub(crate) const fn body_cache(&self) -> &TtlCache {
        let Self { body_cache, .. } = self;
        body_cache
    }

    pub async fn request_token(&self) -> Result<SecretString, VaultError<T::Error>> {
        match &self.settings.auth {
            VaultAuth::Token(token) => Ok(token.clone()),
            VaultAuth::AppRole {
                role_id,
                secret_id,
                mount_path,
            } => {
                if let Some(token) = self.cached_token() {
                    return Ok(token);
                }
                let request = auth::begin_approle_login(self, role_id, secret_id, mount_path)?;
                let limit = request.max_response_bytes;
                let login = auth::finish_login::<T>(
                    self.transport.send(request).await,
                    LoginMethod::AppRole,
                    limit,
                )?;
                self.store_token(login.client_token.clone(), login.lease_duration);
                Ok(login.client_token)
            }
            VaultAuth::TlsCert {
                client_certificate,
                role,
            } => {
                let request = auth::begin_tls_login(self, client_certificate, role.as_deref())?;
                let limit = request.max_response_bytes;
                let login = auth::finish_login::<T>(
                    self.transport.send(request).await,
                    LoginMethod::TlsCert,
                    limit,
                )?;
                Ok(login.client_token)
            }
        }
    }

    pub fn request_token_blocking(&self) -> Result<SecretString, VaultError<T::Error>> {
        match &self.settings.auth {
            VaultAuth::Token(token) => Ok(token.clone()),
            VaultAuth::AppRole {
                role_id,
                secret_id,
                mount_path,
            } => {
                if let Some(token) = self.cached_token() {
                    return Ok(token);
                }
                let request = auth::begin_approle_login(self, role_id, secret_id, mount_path)?;
                let limit = request.max_response_bytes;
                let login = auth::finish_login::<T>(
                    self.transport.send_blocking(request),
                    LoginMethod::AppRole,
                    limit,
                )?;
                self.store_token(login.client_token.clone(), login.lease_duration);
                Ok(login.client_token)
            }
            VaultAuth::TlsCert {
                client_certificate,
                role,
            } => {
                let request = auth::begin_tls_login(self, client_certificate, role.as_deref())?;
                let limit = request.max_response_bytes;
                let login = auth::finish_login::<T>(
                    self.transport.send_blocking(request),
                    LoginMethod::TlsCert,
                    limit,
                )?;
                Ok(login.client_token)
            }
        }
    }

    pub fn get_url(
        &self,
        secret_name: &str,
        settings: &VaultSecretSettings,
    ) -> Result<String, UnsafeSecretNameError> {
        raise_if_unsafe_secret_name(secret_name)?;
        let manager_namespace = self.vault_secret_namespace();
        let namespace_source = settings.namespace.as_deref().or(manager_namespace);
        let namespace = target::sanitize_path_component(namespace_source);
        let mount_source = settings
            .mount
            .as_deref()
            .unwrap_or(self.settings.mount.as_str());
        let mount = target::sanitize_path_component(Some(mount_source))
            .unwrap_or_else(|| DEFAULT_MOUNT.to_owned());
        let manager_prefix = self.settings.path_prefix.as_deref();
        let prefix_source = settings.path_prefix.as_deref().or(manager_prefix);
        let prefix = target::sanitize_path_component(prefix_source);
        let mut url = format!("{}/v1/", self.vault_addr());
        if let Some(segment) = namespace {
            url.push_str(&segment);
            url.push('/');
        }
        url.push_str(&mount);
        url.push_str("/data/");
        if let Some(segment) = prefix {
            url.push_str(&segment);
            url.push('/');
        }
        url.push_str(secret_name);
        Ok(url)
    }

    pub fn build_secret_target(
        &self,
        secret_name: &str,
        params: Option<&VaultSecretParams>,
    ) -> Result<VaultSecretTarget, UnsafeSecretNameError> {
        let empty = VaultSecretSettings::default();
        let settings = params.map_or(&empty, |provided| {
            provided
                .secret_manager_settings
                .as_ref()
                .unwrap_or(&provided.flat)
        });
        let url = self.get_url(secret_name, settings)?;
        let data_key = settings
            .data
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        Ok(VaultSecretTarget {
            url,
            data_key: data_key.unwrap_or("key").to_owned(),
            secret_name: secret_name.to_owned(),
        })
    }

    fn cached_token(&self) -> Option<SecretString> {
        let now = (self.clock)();
        let slot = self.token.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = slot
            .as_ref()
            .filter(|entry| now <= entry.expiry)
            .map(|entry| entry.token.clone());
        drop(slot);
        cached
    }

    fn store_token(&self, token: SecretString, lease_seconds: u64) {
        let Some(expiry) = (self.clock)().checked_add(Duration::from_secs(lease_seconds)) else {
            return;
        };
        let mut slot = self.token.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = Some(CachedToken { token, expiry });
    }
}
