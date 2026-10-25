#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    DISABLE_KEYRING_ENV_VAR, Erase, KEYRING_ACCOUNT, KEYRING_PREFLIGHT_ACCOUNT, KeyringApi,
    KeyringApiError, KeyringUnusable, KeyringVault, Read, SecretVault as _, Write,
};

#[derive(Debug)]
enum Call {
    Block(SyncSender<()>, Receiver<()>),
    Panic,
    Get(Result<Option<&'static str>, KeyringApiError>),
    Set(&'static str, Result<(), KeyringApiError>),
    Delete(&'static str, Result<(), KeyringApiError>),
}

struct Script(Mutex<VecDeque<Call>>);

impl Script {
    fn next(&self, account: &str, password: Option<&SecretString>) -> Call {
        let call = self.0.lock().unwrap().pop_front().unwrap();
        match &call {
            Call::Get(_) => assert_eq!(account, KEYRING_ACCOUNT),
            Call::Set(expected, _) => {
                assert_eq!(account, *expected);
                assert_eq!(
                    password.unwrap().expose_secret(),
                    if account == KEYRING_ACCOUNT {
                        "secret"
                    } else {
                        "preflight"
                    }
                );
            }
            Call::Delete(expected, _) => assert_eq!(account, *expected),
            Call::Block(entered, release) => {
                assert_eq!(account, KEYRING_PREFLIGHT_ACCOUNT);
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
            Call::Panic => panic!("backend panicked"),
        }
        call
    }
}

impl KeyringApi for Script {
    fn get_password(&self, account: &str) -> Result<Option<SecretString>, KeyringApiError> {
        if let Call::Get(result) = self.next(account, None) {
            result.map(|value| value.map(SecretString::from))
        } else {
            Err(KeyringApiError::Unavailable)
        }
    }

    fn set_password(&self, account: &str, password: &SecretString) -> Result<(), KeyringApiError> {
        match self.next(account, Some(password)) {
            Call::Set(_, result) => result,
            Call::Block(..) => Ok(()),
            Call::Get(_) | Call::Delete(..) | Call::Panic => Err(KeyringApiError::Unavailable),
        }
    }

    fn delete_password(&self, account: &str) -> Result<(), KeyringApiError> {
        if let Call::Delete(_, result) = self.next(account, None) {
            result
        } else {
            Err(KeyringApiError::Unavailable)
        }
    }
}

fn scripted(calls: Vec<Call>) -> (KeyringVault, Arc<Script>) {
    let script = Arc::new(Script(Mutex::new(VecDeque::from(calls))));
    let shared = Arc::clone(&script);
    let api: Arc<dyn KeyringApi> = shared;
    let vault = KeyringVault::with_api(Ok(api), Duration::from_secs(5), &BTreeMap::new());
    (vault, script)
}

#[rstest]
#[case::found(Some("secret"))]
#[case::empty(Some(""))]
#[case::missing(None)]
fn read_found_and_missing(#[case] value: Option<&'static str>) {
    let (vault, script) = scripted(vec![Call::Get(Ok(value))]);
    match vault.read().unwrap() {
        Read::SecretFound(secret) => assert_eq!(Some(secret.expose_secret()), value),
        Read::SecretMissing => assert_eq!(value, None),
    }
    assert!(script.0.lock().unwrap().is_empty());
}

#[test]
fn read_exception_unreachable() {
    let (vault, _) = scripted(vec![Call::Get(Err(KeyringApiError::Unavailable))]);
    assert!(matches!(
        vault.read(),
        Err(KeyringUnusable::KeyringUnreachable)
    ));
}

#[rstest]
#[case::one(" 1 ")]
#[case::true_value(" tRuE ")]
#[case::yes("YES")]
#[case::on("on")]
fn read_import_missing_and_disabled(#[case] value: &str) {
    let environment = BTreeMap::from([(DISABLE_KEYRING_ENV_VAR.to_owned(), value.to_owned())]);
    let mut vault = KeyringVault::new(&environment);
    assert!(matches!(
        vault.read(),
        Err(KeyringUnusable::KeyringDisabled)
    ));
    assert_eq!(
        vault.write(&SecretString::from("secret")),
        Err(KeyringUnusable::KeyringDisabled)
    );
    assert_eq!(vault.erase(), Err(KeyringUnusable::KeyringDisabled));
}

#[test]
fn unavailable_without_feature() {
    if !cfg!(feature = "os-keyring") {
        let mut vault = KeyringVault::new(&BTreeMap::new());
        assert!(matches!(
            vault.read(),
            Err(KeyringUnusable::KeyringNotInstalled)
        ));
        assert_eq!(
            vault.write(&SecretString::from("secret")),
            Err(KeyringUnusable::KeyringNotInstalled)
        );
        assert_eq!(vault.erase(), Err(KeyringUnusable::KeyringNotInstalled));
    }
}

#[rstest]
#[case::write_stored_after_readback(Ok(Some("secret")), Ok(Write::SecretStored))]
#[case::write_readback_mismatch_discards_writes(
    Ok(Some("other")),
    Ok(Write::KeyringDiscardsWrites)
)]
#[case::null_backend(Ok(None), Ok(Write::KeyringDiscardsWrites))]
#[case::readback_exception(
    Err(KeyringApiError::Unavailable),
    Err(KeyringUnusable::KeyringUnreachable)
)]
fn verified_write(
    #[case] readback: Result<Option<&'static str>, KeyringApiError>,
    #[case] expected: Result<Write, KeyringUnusable>,
) {
    let (mut vault, script) = scripted(vec![
        Call::Set(KEYRING_PREFLIGHT_ACCOUNT, Ok(())),
        Call::Delete(KEYRING_PREFLIGHT_ACCOUNT, Ok(())),
        Call::Set(KEYRING_ACCOUNT, Ok(())),
        Call::Get(readback),
    ]);
    assert_eq!(vault.write(&SecretString::from("secret")), expected);
    assert!(script.0.lock().unwrap().is_empty());
}

#[rstest]
#[case::write_preflight_delete_failure_suppressed(Ok(()), Err(KeyringApiError::Unavailable))]
#[case::preflight_refusal_is_an_answer(Err(KeyringApiError::Unavailable), Ok(()))]
fn preflight_errors(
    #[case] probe: Result<(), KeyringApiError>,
    #[case] cleanup: Result<(), KeyringApiError>,
) {
    let (mut vault, script) = scripted(vec![
        Call::Set(KEYRING_PREFLIGHT_ACCOUNT, probe),
        Call::Delete(KEYRING_PREFLIGHT_ACCOUNT, cleanup),
        Call::Set(KEYRING_ACCOUNT, Ok(())),
        Call::Get(Ok(Some("secret"))),
    ]);
    assert_eq!(
        vault.write(&SecretString::from("secret")),
        Ok(Write::SecretStored)
    );
    assert!(script.0.lock().unwrap().is_empty());
}

#[test]
fn write_exception_in_set() {
    let (mut vault, script) = scripted(vec![
        Call::Set(KEYRING_PREFLIGHT_ACCOUNT, Ok(())),
        Call::Delete(KEYRING_PREFLIGHT_ACCOUNT, Ok(())),
        Call::Set(KEYRING_ACCOUNT, Err(KeyringApiError::Unavailable)),
    ]);
    assert_eq!(
        vault.write(&SecretString::from("secret")),
        Err(KeyringUnusable::KeyringUnreachable)
    );
    assert!(script.0.lock().unwrap().is_empty());
}

#[rstest]
#[case::erase_confirms_deletion_after_found(Ok(()), Erase::SecretErased)]
#[case::erase_stranded_on_delete_failure(Err(KeyringApiError::Unavailable), Erase::SecretStranded)]
fn erase_found(#[case] deletion: Result<(), KeyringApiError>, #[case] expected: Erase) {
    let (mut vault, script) = scripted(vec![
        Call::Get(Ok(Some("secret"))),
        Call::Delete(KEYRING_ACCOUNT, deletion),
    ]);
    assert_eq!(vault.erase(), Ok(expected));
    assert!(script.0.lock().unwrap().is_empty());
}

#[test]
fn erase_missing_already_gone() {
    let (mut vault, script) = scripted(vec![Call::Get(Ok(None))]);
    assert_eq!(vault.erase(), Ok(Erase::SecretErased));
    assert!(script.0.lock().unwrap().is_empty());
}

#[rstest]
#[case::missing(KeyringUnusable::KeyringNotInstalled)]
#[case::disabled(KeyringUnusable::KeyringDisabled)]
#[case::unreachable(KeyringUnusable::KeyringUnreachable)]
fn write_disabled_missing_and_cache(#[case] reason: KeyringUnusable) {
    let mut vault = KeyringVault::with_api(Err(reason), Duration::ZERO, &BTreeMap::new());
    assert!(matches!(vault.read(), Err(actual) if actual == reason));
    assert_eq!(vault.write(&SecretString::from("secret")), Err(reason));
    assert_eq!(vault.erase(), Err(reason));
}

#[test]
fn erase_unusable_never_tried() {
    let (mut vault, script) = scripted(vec![Call::Get(Err(KeyringApiError::Unavailable))]);
    assert_eq!(vault.erase(), Err(KeyringUnusable::KeyringUnreachable));
    assert!(script.0.lock().unwrap().is_empty());
}

#[test]
fn write_preflight_timeout_cached() {
    let (entered, started) = sync_channel(1);
    let (release, blocked) = sync_channel(1);
    let (mut vault, script) = scripted(vec![Call::Block(entered, blocked)]);
    vault.preflight_timeout = Duration::from_millis(10);
    let secret = SecretString::from("secret");
    let before = Instant::now();
    assert_eq!(
        vault.write(&secret),
        Err(KeyringUnusable::KeyringUnreachable)
    );
    assert!(before.elapsed() < Duration::from_secs(2));
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        vault.read(),
        Err(KeyringUnusable::KeyringUnreachable)
    ));
    assert_eq!(
        vault.write(&secret),
        Err(KeyringUnusable::KeyringUnreachable)
    );
    assert_eq!(vault.erase(), Err(KeyringUnusable::KeyringUnreachable));
    release.send(()).unwrap();
    vault.probe.take().unwrap().join().unwrap();
    assert_eq!(
        vault.write(&secret),
        Err(KeyringUnusable::KeyringUnreachable)
    );
    assert!(script.0.lock().unwrap().is_empty());
}

#[rstest]
#[case::probe(vec![Call::Panic, Call::Delete(KEYRING_PREFLIGHT_ACCOUNT, Ok(())), Call::Set(KEYRING_ACCOUNT, Ok(())), Call::Get(Ok(Some("secret")))], Ok(Write::SecretStored))]
#[case::cleanup(vec![Call::Set(KEYRING_PREFLIGHT_ACCOUNT, Ok(())), Call::Panic, Call::Set(KEYRING_ACCOUNT, Ok(())), Call::Get(Ok(Some("secret")))], Ok(Write::SecretStored))]
#[case::set(vec![Call::Set(KEYRING_PREFLIGHT_ACCOUNT, Ok(())), Call::Delete(KEYRING_PREFLIGHT_ACCOUNT, Ok(())), Call::Panic], Err(KeyringUnusable::KeyringUnreachable))]
#[case::readback(vec![Call::Set(KEYRING_PREFLIGHT_ACCOUNT, Ok(())), Call::Delete(KEYRING_PREFLIGHT_ACCOUNT, Ok(())), Call::Set(KEYRING_ACCOUNT, Ok(())), Call::Panic], Err(KeyringUnusable::KeyringUnreachable))]
fn write_panics(#[case] calls: Vec<Call>, #[case] expected: Result<Write, KeyringUnusable>) {
    let (mut vault, script) = scripted(calls);
    assert_eq!(vault.write(&SecretString::from("secret")), expected);
    assert!(script.0.lock().unwrap().is_empty());
}

#[test]
fn read_and_delete_panics() {
    let (mut vault, script) = scripted(vec![
        Call::Panic,
        Call::Get(Ok(Some("secret"))),
        Call::Panic,
    ]);
    assert!(matches!(
        vault.read(),
        Err(KeyringUnusable::KeyringUnreachable)
    ));
    assert_eq!(vault.erase(), Ok(Erase::SecretStranded));
    assert!(script.0.lock().unwrap().is_empty());
}

#[rstest]
#[case::empty("")]
#[case::false_value("false")]
#[case::off("off")]
#[case::unknown("enabled")]
fn other_values_allow_keyring(#[case] value: &str) {
    let environment = BTreeMap::from([(DISABLE_KEYRING_ENV_VAR.to_owned(), value.to_owned())]);
    let vault = KeyringVault::new(&environment);
    assert!(!matches!(
        vault.backend,
        Err(KeyringUnusable::KeyringDisabled)
    ));
    assert_eq!(vault.preflight_timeout, Duration::from_secs(5));
}

#[cfg(feature = "os-keyring")]
#[test]
fn backend_errors_redact_secret_bytes() {
    let error = KeyringApiError::Native(keyring_core::Error::BadEncoding(b"secret".to_vec()));
    assert_eq!(format!("{error:?}"), "keyring backend failed");
}
