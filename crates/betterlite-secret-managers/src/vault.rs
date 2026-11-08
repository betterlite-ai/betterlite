use std::collections::BTreeMap;
use std::sync::Mutex;

use secrecy::SecretString;
use tokio::time::Instant;

use crate::cyberark::Clock;
use crate::cyberark::ttl_cache::TtlCache;
use crate::secret_name::{UnsafeSecretNameError, raise_if_unsafe_secret_name};
use error::{VaultError, VaultInitError};
use settings::VaultSettings;
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
        let plan = match auth::pending_login(self)? {
            auth::PendingLogin::Ready(token) => return Ok(token),
            auth::PendingLogin::Send(plan) => plan,
        };
        auth::login_outcome(self, plan.method, self.transport.send(plan.request).await)
    }

    pub fn request_token_blocking(&self) -> Result<SecretString, VaultError<T::Error>> {
        let plan = match auth::pending_login(self)? {
            auth::PendingLogin::Ready(token) => return Ok(token),
            auth::PendingLogin::Send(plan) => plan,
        };
        auth::login_outcome(
            self,
            plan.method,
            self.transport.send_blocking(plan.request),
        )
    }

    pub fn get_url(
        &self,
        secret_name: &str,
        settings: &VaultSecretSettings,
    ) -> Result<String, UnsafeSecretNameError> {
        raise_if_unsafe_secret_name(secret_name)?;
        Ok(target::secret_url(self, secret_name, settings))
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
        Ok(VaultSecretTarget {
            url,
            data_key: settings.data_key(),
            secret_name: secret_name.to_owned(),
        })
    }
}
