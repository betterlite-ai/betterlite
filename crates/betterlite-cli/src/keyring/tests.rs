use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Default)]
struct MockKeyring {
    values: Mutex<HashMap<String, String>>,
    fail_get: Mutex<bool>,
    fail_set: Mutex<bool>,
    fail_delete: Mutex<bool>,
    discard_writes: bool,
    block_preflight: bool,
}

impl KeyringApi for MockKeyring {
    fn get_password(
        &self,
        _service: &str,
        account: &str,
    ) -> Result<Option<String>, KeyringApiError> {
        if *self.fail_get.lock().map_err(|_poison| KeyringApiError)? {
            return Err(KeyringApiError);
        }
        Ok(self
            .values
            .lock()
            .map_err(|_poison| KeyringApiError)?
            .get(account)
            .cloned())
    }

    fn set_password(
        &self,
        _service: &str,
        account: &str,
        password: &str,
    ) -> Result<(), KeyringApiError> {
        if *self.fail_set.lock().map_err(|_poison| KeyringApiError)? {
            return Err(KeyringApiError);
        }
        if self.block_preflight && account == KEYRING_PREFLIGHT_ACCOUNT {
            thread::park_timeout(Duration::from_millis(100));
        } else if !self.discard_writes {
            self.values
                .lock()
                .map_err(|_poison| KeyringApiError)?
                .insert(account.to_owned(), password.to_owned());
        } else {
            return Ok(());
        }
        Ok(())
    }

    fn delete_password(&self, _service: &str, account: &str) -> Result<(), KeyringApiError> {
        if *self.fail_delete.lock().map_err(|_poison| KeyringApiError)? {
            return Err(KeyringApiError);
        }
        self.values
            .lock()
            .map_err(|_poison| KeyringApiError)?
            .remove(account);
        Ok(())
    }
}

fn vault(mock: MockKeyring) -> KeyringVault {
    KeyringVault::with_api(Arc::new(mock), Duration::from_millis(20))
}

#[test]
fn read_found_and_missing() {
    let mock = MockKeyring::default();
    mock.values
        .lock()
        .unwrap()
        .insert(KEYRING_ACCOUNT.to_owned(), "secret".to_owned());
    let vault = vault(mock);
    assert_eq!(
        vault.read(),
        SecretRead::SecretFound(SecretString::from("secret"))
    );
    vault.erase();
    assert_eq!(vault.read(), SecretRead::SecretMissing);
}

#[test]
fn read_exception_unreachable() {
    let mock = MockKeyring {
        fail_get: Mutex::new(true),
        ..Default::default()
    };
    assert_eq!(vault(mock).read(), SecretRead::KeyringUnreachable);
}

#[test]
fn write_stored_after_readback_and_discards_writes() {
    assert_eq!(
        vault(MockKeyring::default()).write("secret"),
        SecretWrite::SecretStored
    );
    let discarding_mock = MockKeyring {
        discard_writes: true,
        ..Default::default()
    };
    assert_eq!(
        vault(discarding_mock).write("secret"),
        SecretWrite::KeyringDiscardsWrites
    );
}

#[test]
fn write_preflight_timeout_is_cached() {
    let mock = MockKeyring {
        block_preflight: true,
        ..Default::default()
    };
    let vault = vault(mock);
    let started = Instant::now();
    assert_eq!(vault.write("secret"), SecretWrite::KeyringUnreachable);
    assert!(started.elapsed() < Duration::from_millis(80));
    assert_eq!(vault.write("secret"), SecretWrite::KeyringUnreachable);
}

#[test]
fn write_preflight_delete_failure_is_suppressed() {
    let mock = MockKeyring {
        fail_delete: Mutex::new(true),
        ..Default::default()
    };
    assert_eq!(vault(mock).write("secret"), SecretWrite::SecretStored);
}

#[test]
fn write_and_erase_exception_paths_are_values() {
    let mock = MockKeyring {
        fail_set: Mutex::new(true),
        ..Default::default()
    };
    assert_eq!(vault(mock).write("secret"), SecretWrite::KeyringUnreachable);
    let stranded_mock = MockKeyring {
        fail_delete: Mutex::new(true),
        ..Default::default()
    };
    stranded_mock
        .values
        .lock()
        .unwrap()
        .insert(KEYRING_ACCOUNT.to_owned(), "secret".to_owned());
    assert_eq!(vault(stranded_mock).erase(), SecretErase::SecretStranded);
}

#[test]
fn erase_confirms_deletion_and_missing_is_already_gone() {
    assert_eq!(
        vault(MockKeyring::default()).erase(),
        SecretErase::SecretErased
    );
    let mock = MockKeyring::default();
    mock.values
        .lock()
        .unwrap()
        .insert(KEYRING_ACCOUNT.to_owned(), "secret".to_owned());
    assert_eq!(vault(mock).erase(), SecretErase::SecretErased);
}

#[test]
fn disabled_value_is_trimmed_and_case_insensitive() {
    assert!(keyring_disabled_value(" TrUe "));
    assert!(!keyring_disabled_value("false"));
}

#[test]
fn unavailable_without_feature() {
    if cfg!(not(feature = "os-keyring")) {
        assert_eq!(KeyringVault::new().read(), SecretRead::KeyringNotInstalled);
        assert_eq!(
            KeyringVault::new().write("secret"),
            SecretWrite::KeyringNotInstalled
        );
        assert_eq!(
            KeyringVault::new().erase(),
            SecretErase::KeyringNotInstalled
        );
    }
}
