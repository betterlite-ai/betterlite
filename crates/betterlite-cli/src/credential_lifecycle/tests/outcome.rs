use std::error::Error as _;
use std::io;
use std::path::PathBuf;

use betterlite_logging::private_json::error::PrivateJsonError;

use crate::credential_lifecycle::outcome::{
    CredentialNotCleared, CredentialNotRecorded, CredentialNotSaved, SecretClear, SecretSave,
};
use crate::token_records::error::TokenRecordError;

fn save_outcome_name(outcome: &SecretSave) -> &'static str {
    match outcome {
        SecretSave::SecretStored => "secret_stored",
        SecretSave::KeyringDiscardsWrites => "keyring_discards_writes",
        SecretSave::KeyringDisabled => "keyring_disabled",
        SecretSave::KeyringNotInstalled => "keyring_not_installed",
        SecretSave::KeyringUnreachable => "keyring_unreachable",
        SecretSave::CredentialNotSaved(_) => "credential_not_saved",
        SecretSave::CredentialNotRecorded(_) => "credential_not_recorded",
    }
}

fn clear_outcome_name(outcome: &SecretClear) -> &'static str {
    match outcome {
        SecretClear::SecretErased => "secret_erased",
        SecretClear::SecretStranded => "secret_stranded",
        SecretClear::KeyringDisabled => "keyring_disabled",
        SecretClear::KeyringNotInstalled => "keyring_not_installed",
        SecretClear::KeyringUnreachable => "keyring_unreachable",
        SecretClear::CredentialNotCleared(_) => "credential_not_cleared",
    }
}

fn not_saved() -> CredentialNotSaved {
    CredentialNotSaved {
        source: TokenRecordError::Write {
            source: PrivateJsonError::Commit(
                PathBuf::from("token.json"),
                io::Error::other("read-only file system"),
            ),
        },
    }
}

#[test]
fn save_outcomes_are_the_closed_set() {
    let outcomes = [
        SecretSave::SecretStored,
        SecretSave::KeyringDiscardsWrites,
        SecretSave::KeyringDisabled,
        SecretSave::KeyringNotInstalled,
        SecretSave::KeyringUnreachable,
        SecretSave::CredentialNotSaved(not_saved()),
        SecretSave::CredentialNotRecorded(CredentialNotRecorded),
    ];
    let names: Vec<&str> = outcomes.iter().map(save_outcome_name).collect();
    assert_eq!(
        names,
        [
            "secret_stored",
            "keyring_discards_writes",
            "keyring_disabled",
            "keyring_not_installed",
            "keyring_unreachable",
            "credential_not_saved",
            "credential_not_recorded",
        ]
    );
}

#[test]
fn clear_outcomes_are_the_closed_set() {
    let outcomes = [
        SecretClear::SecretErased,
        SecretClear::SecretStranded,
        SecretClear::KeyringDisabled,
        SecretClear::KeyringNotInstalled,
        SecretClear::KeyringUnreachable,
        SecretClear::CredentialNotCleared(CredentialNotCleared {
            source: io::Error::other("the directory is read-only"),
        }),
    ];
    let names: Vec<&str> = outcomes.iter().map(clear_outcome_name).collect();
    assert_eq!(
        names,
        [
            "secret_erased",
            "secret_stranded",
            "keyring_disabled",
            "keyring_not_installed",
            "keyring_unreachable",
            "credential_not_cleared",
        ]
    );
}

#[test]
fn credential_not_saved_keeps_the_record_error_as_its_source() {
    let failure = not_saved();
    let source = failure.source().expect("a wrapped record error");
    assert!(source.is::<TokenRecordError>());
    assert!(failure.to_string().contains("read-only file system"));
}

#[test]
fn credential_not_cleared_keeps_the_removal_error_as_its_source() {
    let failure = CredentialNotCleared {
        source: io::Error::other("the directory is read-only"),
    };
    let source = failure.source().expect("a wrapped removal error");
    assert!(source.is::<io::Error>());
    assert!(failure.to_string().contains("the directory is read-only"));
}

#[test]
fn credential_not_recorded_compares_by_value() {
    assert_eq!(CredentialNotRecorded, CredentialNotRecorded);
}
