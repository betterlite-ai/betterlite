use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

pub const KEYRING_SERVICE: &str = "litellm-cli";
pub const KEYRING_ACCOUNT: &str = "credential";
pub const KEYRING_PREFLIGHT_ACCOUNT: &str = "credential-preflight";
pub const DISABLE_KEYRING_ENV_VAR: &str = "LITELLM_CLI_DISABLE_KEYRING";

const PREFLIGHT_VALUE: &str = "preflight";
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum SecretRead {
    SecretFound(SecretString),
    SecretMissing,
    KeyringNotInstalled,
    KeyringDisabled,
    KeyringUnreachable,
}

impl PartialEq for SecretRead {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SecretFound(left), Self::SecretFound(right)) => {
                left.expose_secret() == right.expose_secret()
            }
            (Self::SecretMissing, Self::SecretMissing)
            | (Self::KeyringNotInstalled, Self::KeyringNotInstalled)
            | (Self::KeyringDisabled, Self::KeyringDisabled)
            | (Self::KeyringUnreachable, Self::KeyringUnreachable) => true,
            _ => false,
        }
    }
}

impl Eq for SecretRead {}

#[derive(Debug, PartialEq, Eq)]
pub enum SecretWrite {
    SecretStored,
    KeyringNotInstalled,
    KeyringDisabled,
    KeyringUnreachable,
    KeyringDiscardsWrites,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SecretErase {
    SecretErased,
    SecretStranded,
    KeyringNotInstalled,
    KeyringDisabled,
    KeyringUnreachable,
}

pub type KeyringUnusable = SecretRead;

pub trait SecretVault {
    fn read(&self) -> SecretRead;
    fn write(&self, blob: &str) -> SecretWrite;
    fn erase(&self) -> SecretErase;
}

#[derive(Debug, Clone, Copy)]
pub struct KeyringApiError;

pub trait KeyringApi: Send + Sync {
    fn get_password(&self, service: &str, account: &str)
    -> Result<Option<String>, KeyringApiError>;
    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeyringApiError>;
    fn delete_password(&self, service: &str, account: &str) -> Result<(), KeyringApiError>;
}

enum Backend {
    Available(Arc<dyn KeyringApi>),
    NotInstalled,
    Disabled,
}

pub struct KeyringVault {
    backend: Backend,
    preflight_timeout: Duration,
    stopped_answering: AtomicBool,
}

impl KeyringVault {
    #[must_use]
    pub fn new() -> Self {
        let backend = if keyring_disabled() {
            Backend::Disabled
        } else {
            configured_backend()
        };
        Self::from_backend(backend, PREFLIGHT_TIMEOUT)
    }

    pub fn with_api(api: Arc<dyn KeyringApi>, preflight_timeout: Duration) -> Self {
        Self::from_backend(Backend::Available(api), preflight_timeout)
    }

    const fn from_backend(backend: Backend, preflight_timeout: Duration) -> Self {
        Self {
            backend,
            preflight_timeout,
            stopped_answering: AtomicBool::new(false),
        }
    }

    const fn unusable_read(&self) -> Option<SecretRead> {
        match &self.backend {
            Backend::Available(_) => None,
            Backend::NotInstalled => Some(SecretRead::KeyringNotInstalled),
            Backend::Disabled => Some(SecretRead::KeyringDisabled),
        }
    }

    const fn unusable_write(&self) -> Option<SecretWrite> {
        match &self.backend {
            Backend::Available(_) => None,
            Backend::NotInstalled => Some(SecretWrite::KeyringNotInstalled),
            Backend::Disabled => Some(SecretWrite::KeyringDisabled),
        }
    }

    const fn unusable_erase(&self) -> Option<SecretErase> {
        match &self.backend {
            Backend::Available(_) => None,
            Backend::NotInstalled => Some(SecretErase::KeyringNotInstalled),
            Backend::Disabled => Some(SecretErase::KeyringDisabled),
        }
    }

    fn api(&self) -> Option<Arc<dyn KeyringApi>> {
        match &self.backend {
            Backend::Available(api) => Some(Arc::clone(api)),
            Backend::NotInstalled | Backend::Disabled => None,
        }
    }

    fn preflight(&self, api: Arc<dyn KeyringApi>) -> bool {
        let (sender, receiver) = sync_channel(1);
        match thread::Builder::new()
            .name("betterlite-keyring-preflight".to_owned())
            .spawn(move || {
                let answered = catch_unwind(AssertUnwindSafe(|| {
                    api.set_password(KEYRING_SERVICE, KEYRING_PREFLIGHT_ACCOUNT, PREFLIGHT_VALUE)
                        .is_ok()
                }))
                .unwrap_or(false);
                match sender.send(answered) {
                    Ok(()) | Err(_) => {}
                }
            }) {
            Ok(_) | Err(_) => {}
        }
        receiver.recv_timeout(self.preflight_timeout).is_ok()
    }

    fn remove_preflight(api: &dyn KeyringApi) {
        drop(catch_unwind(AssertUnwindSafe(|| {
            match api.delete_password(KEYRING_SERVICE, KEYRING_PREFLIGHT_ACCOUNT) {
                Ok(()) | Err(_) => {}
            }
        })));
    }
}

impl Default for KeyringVault {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretVault for KeyringVault {
    fn read(&self) -> SecretRead {
        if self.stopped_answering.load(Ordering::Acquire) {
            return SecretRead::KeyringUnreachable;
        }
        if let Some(result) = self.unusable_read() {
            return result;
        }
        let Some(api) = self.api() else {
            return SecretRead::KeyringNotInstalled;
        };
        match catch_unwind(AssertUnwindSafe(|| {
            api.get_password(KEYRING_SERVICE, KEYRING_ACCOUNT)
        })) {
            Ok(Ok(Some(blob))) => SecretRead::SecretFound(SecretString::from(blob)),
            Ok(Ok(None)) => SecretRead::SecretMissing,
            Ok(Err(_)) | Err(_) => SecretRead::KeyringUnreachable,
        }
    }

    fn write(&self, blob: &str) -> SecretWrite {
        if self.stopped_answering.load(Ordering::Acquire) {
            return SecretWrite::KeyringUnreachable;
        }
        if let Some(result) = self.unusable_write() {
            return result;
        }
        let Some(api) = self.api() else {
            return SecretWrite::KeyringNotInstalled;
        };
        if !self.preflight(Arc::clone(&api)) {
            self.stopped_answering.store(true, Ordering::Release);
            return SecretWrite::KeyringUnreachable;
        }
        Self::remove_preflight(api.as_ref());
        let stored = catch_unwind(AssertUnwindSafe(|| {
            api.set_password(KEYRING_SERVICE, KEYRING_ACCOUNT, blob)
        }));
        if !matches!(stored, Ok(Ok(()))) {
            return SecretWrite::KeyringUnreachable;
        }
        match self.read() {
            SecretRead::SecretFound(found) if found.expose_secret() == blob => {
                SecretWrite::SecretStored
            }
            SecretRead::KeyringUnreachable => SecretWrite::KeyringUnreachable,
            SecretRead::SecretFound(_) | SecretRead::SecretMissing => {
                SecretWrite::KeyringDiscardsWrites
            }
            SecretRead::KeyringNotInstalled | SecretRead::KeyringDisabled => {
                SecretWrite::KeyringUnreachable
            }
        }
    }

    fn erase(&self) -> SecretErase {
        if self.stopped_answering.load(Ordering::Acquire) {
            return SecretErase::KeyringUnreachable;
        }
        if let Some(result) = self.unusable_erase() {
            return result;
        }
        match self.read() {
            SecretRead::SecretMissing => return SecretErase::SecretErased,
            SecretRead::KeyringNotInstalled => return SecretErase::KeyringNotInstalled,
            SecretRead::KeyringDisabled => return SecretErase::KeyringDisabled,
            SecretRead::KeyringUnreachable => return SecretErase::KeyringUnreachable,
            SecretRead::SecretFound(_) => {}
        }
        let Some(api) = self.api() else {
            return SecretErase::KeyringNotInstalled;
        };
        let deleted = catch_unwind(AssertUnwindSafe(|| {
            api.delete_password(KEYRING_SERVICE, KEYRING_ACCOUNT)
        }));
        if !matches!(deleted, Ok(Ok(()))) {
            return SecretErase::SecretStranded;
        }
        match self.read() {
            SecretRead::SecretMissing => SecretErase::SecretErased,
            SecretRead::KeyringUnreachable => SecretErase::KeyringUnreachable,
            SecretRead::SecretFound(_) => SecretErase::SecretStranded,
            SecretRead::KeyringNotInstalled | SecretRead::KeyringDisabled => {
                SecretErase::SecretStranded
            }
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the task-defined kill switch is an environment variable"
)]
fn keyring_disabled() -> bool {
    use std::env::var as environment_value;

    environment_value(DISABLE_KEYRING_ENV_VAR).is_ok_and(|value| keyring_disabled_value(&value))
}

fn keyring_disabled_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(feature = "os-keyring")]
fn configured_backend() -> Backend {
    Backend::Available(Arc::new(OsKeyringApi))
}

#[cfg(not(feature = "os-keyring"))]
const fn configured_backend() -> Backend {
    Backend::NotInstalled
}

#[cfg(feature = "os-keyring")]
struct OsKeyringApi;

#[cfg(feature = "os-keyring")]
impl KeyringApi for OsKeyringApi {
    fn get_password(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, KeyringApiError> {
        let entry = keyring::Entry::new(service, account).map_err(|_| KeyringApiError)?;
        match entry.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(KeyringApiError),
        }
    }

    fn set_password(
        &self,
        service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeyringApiError> {
        keyring::Entry::new(service, account)
            .map_err(|_| KeyringApiError)?
            .set_password(password)
            .map_err(|_| KeyringApiError)
    }

    fn delete_password(&self, service: &str, account: &str) -> Result<(), KeyringApiError> {
        keyring::Entry::new(service, account)
            .map_err(|_| KeyringApiError)?
            .delete_credential()
            .map_err(|_| KeyringApiError)
    }
}

#[cfg(test)]
mod tests;
