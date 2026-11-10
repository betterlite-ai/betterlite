use secrecy::ExposeSecret as _;

use crate::credential_lifecycle::clear::clear_cli_token;
use crate::credential_lifecycle::outcome::SecretClear;
use crate::keyring::KeyringUnusable;

use rstest::rstest;

use super::support::{
    Refusal, SERVER, StageRefusal, VaultRefusal, fake_vault, metadata, record, scripted_files,
    vault_entry,
};

#[tokio::test]
async fn clear_erases_both_stores() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(SERVER, "sk-secret", None, 1_000.0));

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::SecretErased));
    assert!(!files.exists());
    assert!(vault.blob.is_none());
    assert_eq!(vault.erases(), 1);
}

#[tokio::test]
async fn clear_reports_a_stranded_secret_and_still_ends_the_local_login() {
    let files = scripted_files();
    files.put(&metadata(1_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(SERVER, "sk-secret", None, 1_000.0));
    vault.refusals.push(VaultRefusal::Unerasable);

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::SecretStranded));
    assert!(!files.exists());
    assert!(vault.blob.is_some());
}

#[tokio::test]
async fn clear_keeps_a_scrubbed_note_when_the_keychain_cannot_be_reached() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::KeyringUnreachable));
    let note = files.stored().expect("the unreachable-keychain note stays");
    assert!(note.key.is_none());
    assert!(note.refresh_token.is_none());
    assert_eq!(note.jwt_token.expose_secret(), "");
    assert_eq!(note.user_id, "user-1");
}

#[tokio::test]
async fn clear_scrubs_the_note_in_place_when_no_replacement_is_accepted() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.stage = StageRefusal::Always;
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::KeyringUnreachable));
    let note = files.stored().expect("the note stays");
    assert!(note.key.is_none());
    assert!(note.refresh_token.is_none());
}

#[tokio::test]
async fn clear_writes_the_note_again_after_the_removal_took_it() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.stage = StageRefusal::WhilePresent;
    files.refusals.push(Refusal::Overwrite);
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::KeyringUnreachable));
    let note = files.stored().expect("the note is written again");
    assert!(note.key.is_none());
    assert!(note.refresh_token.is_none());
}

#[tokio::test]
async fn clear_reports_not_cleared_when_the_removal_and_the_rewrite_both_fail() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.stage = StageRefusal::Always;
    files.refusals.push(Refusal::Overwrite);
    files.refusals.push(Refusal::Remove);
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::CredentialNotCleared(_)));
    let kept = files.stored().expect("the file is left untouched");
    assert_eq!(
        kept.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
}

#[tokio::test]
async fn clear_succeeds_on_a_machine_that_never_logged_in() {
    let files = scripted_files();
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::SecretErased));
    assert_eq!(vault.erases(), 1);
}

#[tokio::test]
async fn clear_keeps_reporting_the_keychain_it_could_not_clear() {
    let files = scripted_files();
    files.put(&metadata(1_000.0));
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let first = clear_cli_token(&files, &mut vault).await;
    let second = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(first, SecretClear::KeyringUnreachable));
    assert!(matches!(second, SecretClear::KeyringUnreachable));
    assert!(files.exists());
}

#[tokio::test]
async fn clear_does_not_alarm_over_a_metadata_file_it_cannot_remove() {
    let mut files = scripted_files();
    files.put(&metadata(1_000.0));
    files.refusals.push(Refusal::Remove);
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(SERVER, "sk-secret", None, 1_000.0));

    let outcome = clear_cli_token(&files, &mut vault).await;

    assert!(matches!(outcome, SecretClear::SecretErased));
    assert!(files.exists());
    assert!(vault.blob.is_none());
}

#[rstest]
#[case::not_installed(KeyringUnusable::KeyringNotInstalled, "keyring_not_installed")]
#[case::disabled(KeyringUnusable::KeyringDisabled, "keyring_disabled")]
#[case::unreachable(KeyringUnusable::KeyringUnreachable, "keyring_unreachable")]
#[tokio::test]
async fn clear_takes_the_file_secret_away_from_a_note_it_keeps(
    #[case] failure: KeyringUnusable,
    #[case] expected: &str,
) {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);
    vault.failure = failure;

    let outcome = clear_cli_token(&files, &mut vault).await;

    let reported = match outcome {
        SecretClear::KeyringNotInstalled => "keyring_not_installed",
        SecretClear::KeyringDisabled => "keyring_disabled",
        SecretClear::KeyringUnreachable => "keyring_unreachable",
        SecretClear::SecretErased => "secret_erased",
        SecretClear::SecretStranded => "secret_stranded",
        SecretClear::CredentialNotCleared(_) => "credential_not_cleared",
    };
    assert_eq!(reported, expected);
    let note = files.stored().expect("the note stays");
    assert!(note.key.is_none());
    assert!(note.refresh_token.is_none());
}
