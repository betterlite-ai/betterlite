use keyring_core::api::CredentialStoreApi as _;
use secrecy::{ExposeSecret as _, SecretString};

use super::{KEYRING_SERVICE, KeyringApi, KeyringApiError};

pub(super) struct Api;

fn entry(service: &str, account: &str) -> Result<keyring_core::Entry, KeyringApiError> {
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(target_os = "linux")]
    let store = dbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new();
    let ready = store.map_err(KeyringApiError::Native)?;
    ready
        .build(service, account, None)
        .map_err(KeyringApiError::Native)
}

impl KeyringApi for Api {
    fn get_password(&self, account: &str) -> Result<Option<SecretString>, KeyringApiError> {
        match entry(KEYRING_SERVICE, account)?.get_password() {
            Ok(password) => Ok(Some(SecretString::from(password))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(source) => Err(KeyringApiError::Native(source)),
        }
    }

    fn set_password(&self, account: &str, password: &SecretString) -> Result<(), KeyringApiError> {
        entry(KEYRING_SERVICE, account)?
            .set_password(password.expose_secret())
            .map_err(KeyringApiError::Native)
    }

    fn delete_password(&self, account: &str) -> Result<(), KeyringApiError> {
        entry(KEYRING_SERVICE, account)?
            .delete_credential()
            .map_err(KeyringApiError::Native)
    }
}
