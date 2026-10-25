use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::sync_channel;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use thiserror::Error;

pub const KEYRING_SERVICE: &str = "litellm-cli";
pub const KEYRING_ACCOUNT: &str = "credential";
pub const KEYRING_PREFLIGHT_ACCOUNT: &str = "credential-preflight";
pub const DISABLE_KEYRING_ENV_VAR: &str = "LITELLM_CLI_DISABLE_KEYRING";

#[derive(Debug)]
pub enum Read {
    SecretFound(SecretString),
    SecretMissing,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Write {
    SecretStored,
    KeyringDiscardsWrites,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Erase {
    SecretErased,
    SecretStranded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyringUnusable {
    KeyringNotInstalled,
    KeyringDisabled,
    KeyringUnreachable,
}

pub type SecretRead = Result<Read, KeyringUnusable>;
pub type SecretWrite = Result<Write, KeyringUnusable>;
pub type SecretErase = Result<Erase, KeyringUnusable>;

pub trait SecretVault {
    fn read(&self) -> SecretRead;
    fn write(&mut self, blob: &SecretString) -> SecretWrite;
    fn erase(&mut self) -> SecretErase;
}

#[derive(Error)]
pub enum KeyringApiError {
    #[error("keyring backend unavailable")]
    Unavailable,
    #[cfg(feature = "os-keyring")]
    #[error("keyring backend failed")]
    Native(#[source] keyring_core::Error),
}

impl fmt::Debug for KeyringApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Backend encoding errors can carry the secret bytes in their Debug output.
        fmt::Display::fmt(self, f)
    }
}

pub trait KeyringApi: Send + Sync {
    fn get_password(&self, account: &str) -> Result<Option<SecretString>, KeyringApiError>;
    fn set_password(&self, account: &str, password: &SecretString) -> Result<(), KeyringApiError>;
    fn delete_password(&self, account: &str) -> Result<(), KeyringApiError>;
}

pub struct KeyringVault {
    backend: Result<Arc<dyn KeyringApi>, KeyringUnusable>,
    preflight_timeout: Duration,
    probe: Option<JoinHandle<()>>,
}

impl KeyringVault {
    pub fn new(environment: &dyn EnvironmentLookup) -> Self {
        #[cfg(feature = "os-keyring")]
        let backend: Result<Arc<dyn KeyringApi>, KeyringUnusable> = Ok(Arc::new(native::Api));
        #[cfg(not(feature = "os-keyring"))]
        let backend = Err(KeyringUnusable::KeyringNotInstalled);
        Self::with_api(backend, Duration::from_secs(5), environment)
    }

    pub fn with_api(
        backend: Result<Arc<dyn KeyringApi>, KeyringUnusable>,
        preflight_timeout: Duration,
        environment: &dyn EnvironmentLookup,
    ) -> Self {
        let disabled = environment
            .lookup(DISABLE_KEYRING_ENV_VAR)
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        Self {
            backend: if disabled {
                Err(KeyringUnusable::KeyringDisabled)
            } else {
                backend
            },
            preflight_timeout,
            probe: None,
        }
    }

    fn preflight(&mut self, api: Arc<dyn KeyringApi>) -> Result<(), KeyringUnusable> {
        let (sender, receiver) = sync_channel(1);
        self.probe = thread::Builder::new()
            .name("betterlite-keyring-preflight".to_owned())
            .spawn(move || {
                // Even a refused probe proves that the backend answered.
                drop(catch_unwind(AssertUnwindSafe(|| {
                    api.set_password(KEYRING_PREFLIGHT_ACCOUNT, &SecretString::from("preflight"))
                })));
                let _closed = sender.send(());
            })
            .ok();
        if receiver.recv_timeout(self.preflight_timeout).is_err() {
            self.backend = Err(KeyringUnusable::KeyringUnreachable);
            return Err(KeyringUnusable::KeyringUnreachable);
        }
        if let Some(probe) = self.probe.take() {
            let _finished = probe.join();
        }
        Ok(())
    }
}

impl SecretVault for KeyringVault {
    fn read(&self) -> SecretRead {
        let api = self.backend.as_ref().map_err(|reason| *reason)?;
        match catch_unwind(AssertUnwindSafe(|| api.get_password(KEYRING_ACCOUNT))) {
            Ok(Ok(Some(blob))) => Ok(Read::SecretFound(blob)),
            Ok(Ok(None)) => Ok(Read::SecretMissing),
            Ok(Err(_)) | Err(_) => Err(KeyringUnusable::KeyringUnreachable),
        }
    }

    fn write(&mut self, blob: &SecretString) -> SecretWrite {
        let api = Arc::clone(self.backend.as_ref().map_err(|reason| *reason)?);
        self.preflight(Arc::clone(&api))?;
        // The probe contains no secret, so cleanup failure must not prevent the real write.
        drop(catch_unwind(AssertUnwindSafe(|| {
            api.delete_password(KEYRING_PREFLIGHT_ACCOUNT)
        })));
        let stored = catch_unwind(AssertUnwindSafe(|| api.set_password(KEYRING_ACCOUNT, blob)));
        if !matches!(stored, Ok(Ok(()))) {
            return Err(KeyringUnusable::KeyringUnreachable);
        }
        match self.read()? {
            Read::SecretFound(found)
                if bool::from(
                    found
                        .expose_secret()
                        .as_bytes()
                        .ct_eq(blob.expose_secret().as_bytes()),
                ) =>
            {
                Ok(Write::SecretStored)
            }
            Read::SecretFound(_) | Read::SecretMissing => Ok(Write::KeyringDiscardsWrites),
        }
    }

    fn erase(&mut self) -> SecretErase {
        if matches!(self.read()?, Read::SecretMissing) {
            return Ok(Erase::SecretErased);
        }
        let api = self.backend.as_ref().map_err(|reason| *reason)?;
        let deleted = catch_unwind(AssertUnwindSafe(|| api.delete_password(KEYRING_ACCOUNT)));
        Ok(if matches!(deleted, Ok(Ok(()))) {
            Erase::SecretErased
        } else {
            Erase::SecretStranded
        })
    }
}

#[cfg(feature = "os-keyring")]
mod native;
mod tests;
