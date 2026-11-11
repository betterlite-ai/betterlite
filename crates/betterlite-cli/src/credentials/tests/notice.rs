use std::io;
use std::path::Path;

use betterlite_logging::private_json::error::PrivateJsonError;

use crate::credential_lifecycle::outcome::{CredentialNotRecorded, CredentialNotSaved, SecretSave};
use crate::credentials::notice::{
    KEYRING_ENABLE_HINT, KEYRING_INSTALL_HINT, STRANDED_CREDENTIAL_MESSAGE,
    UNCHECKED_KEYCHAIN_MESSAGE, keychain_unreadable_notice, storage_notice,
};
use crate::keyring::KeyringUnusable;
use crate::token_records::error::TokenRecordError;

use super::support::{FakeVault, secret};

const TOKEN_PATH: &str = "/home/tester/.litellm/token.json";

fn token_path() -> &'static Path {
    Path::new(TOKEN_PATH)
}

fn not_saved() -> SecretSave {
    SecretSave::CredentialNotSaved(CredentialNotSaved {
        source: TokenRecordError::Write {
            source: PrivateJsonError::Commit(
                token_path().to_path_buf(),
                io::Error::other("denied"),
            ),
        },
    })
}

fn assert_notice(outcome: &SecretSave, expected: &str) {
    assert_eq!(storage_notice(outcome, token_path()), expected);
}

#[test]
fn storage_notice_reports_an_os_keychain_store() {
    assert_notice(
        &SecretSave::SecretStored,
        "Credential stored in your OS keychain.",
    );
}

#[test]
fn storage_notice_points_at_the_file_when_the_keyring_package_is_missing() {
    assert_notice(
        &SecretSave::KeyringNotInstalled,
        &format!(
            "Credential stored in {TOKEN_PATH} (owner-only). For OS keychain storage, install the keyring package with: pip install 'litellm[cli]'"
        ),
    );
}

#[test]
fn storage_notice_names_the_disabled_keychain_variable() {
    assert_notice(
        &SecretSave::KeyringDisabled,
        &format!(
            "Keychain storage is off (LITELLM_CLI_DISABLE_KEYRING). Credential stored in {TOKEN_PATH} (owner-only)."
        ),
    );
}

#[test]
fn storage_notice_reports_an_unreachable_keychain() {
    assert_notice(
        &SecretSave::KeyringUnreachable,
        &format!("No OS keychain available. Credential stored in {TOKEN_PATH} (owner-only)."),
    );
}

#[test]
fn storage_notice_explains_a_keyring_that_discards_writes() {
    assert_notice(
        &SecretSave::KeyringDiscardsWrites,
        &format!(
            "Your keyring backend keeps nothing it is given, so the credential was stored in {TOKEN_PATH} (owner-only) instead. For OS keychain storage, run: keyring --enable (or unset PYTHON_KEYRING_BACKEND)"
        ),
    );
}

#[test]
fn storage_notice_carries_the_failure_that_saved_nothing() {
    assert_notice(
        &not_saved(),
        &format!(
            "Signed in, but the credential could not be saved to {TOKEN_PATH}: cannot write the token record: cannot commit a staged private file into {TOKEN_PATH}: denied. Any login you already had is untouched. Run 'lite login' again once that path is writable, or 'lite logout' to clear whatever is stored now."
        ),
    );
}

#[test]
fn storage_notice_explains_a_keychain_entry_the_file_does_not_name() {
    assert_notice(
        &SecretSave::CredentialNotRecorded(CredentialNotRecorded),
        &format!(
            "Signed in, and the credential is in your OS keychain, but {TOKEN_PATH} could not be replaced, so it still describes your previous login and may still hold its credential. Run 'lite login' again once that path is writable, or 'lite logout' to clear both."
        ),
    );
}

#[test]
fn keychain_unreadable_notice_explains_a_missing_keyring_package() {
    let vault = FakeVault::unavailable(KeyringUnusable::KeyringNotInstalled);

    assert_eq!(
        keychain_unreadable_notice(&vault),
        "Your credential is in your OS keychain, which this install cannot read without the keyring package. Install it with: pip install 'litellm[cli]', or run 'lite login' to start over.".to_owned()
    );
}

#[test]
fn keychain_unreadable_notice_names_the_disabled_keychain_variable() {
    let vault = FakeVault::unavailable(KeyringUnusable::KeyringDisabled);

    assert_eq!(
        keychain_unreadable_notice(&vault),
        "Your credential is in your OS keychain, which LITELLM_CLI_DISABLE_KEYRING is blocking. Unset it, or run 'lite login' to start over."
    );
}

#[test]
fn keychain_unreadable_notice_explains_an_unreachable_keychain() {
    let vault = FakeVault::unavailable(KeyringUnusable::KeyringUnreachable);

    assert_eq!(
        keychain_unreadable_notice(&vault),
        "Your credential is in your OS keychain, which could not be read. Unlock it, or run 'lite login' to start over."
    );
}

#[test]
fn keychain_unreadable_notice_covers_a_readable_or_absent_entry() {
    let mut found = FakeVault::working();
    found.blob = Some(secret("sk-secret"));
    let missing = FakeVault::working();

    assert_eq!(
        keychain_unreadable_notice(&found),
        "Your credential could not be read from your OS keychain. Run 'lite login' to start over."
    );
    assert_eq!(
        keychain_unreadable_notice(&missing),
        "Your credential could not be read from your OS keychain. Run 'lite login' to start over."
    );
}

#[test]
fn the_keyring_hints_and_logout_messages_keep_lite_llm_text() {
    assert_eq!(KEYRING_INSTALL_HINT, "pip install 'litellm[cli]'");
    assert_eq!(
        KEYRING_ENABLE_HINT,
        "keyring --enable (or unset PYTHON_KEYRING_BACKEND)"
    );
    assert_eq!(
        STRANDED_CREDENTIAL_MESSAGE,
        "Logged out locally, but your credential is still in the OS keychain and could not be removed."
    );
    assert_eq!(
        UNCHECKED_KEYCHAIN_MESSAGE,
        "Logged out locally, but your OS keychain could not be checked, so a credential stored there by an earlier login may still be usable."
    );
}
