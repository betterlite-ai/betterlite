use secrecy::ExposeSecret as _;

use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::keyring::KeyringUnusable;
use crate::token_records::secret::decode_secret;

use rstest::rstest;

use super::support::{
    Refusal, SERVER, StageRefusal, VaultRefusal, encoded, fake_vault, metadata, record,
    scripted_files, secret,
};

#[tokio::test]
async fn save_puts_the_secret_in_the_vault_and_keeps_the_file_secret_free() {
    let files = scripted_files();
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, record(1_000.0)).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let stored = files.stored().expect("the metadata file");
    assert!(stored.key.is_none());
    assert_eq!(stored.jwt_token.expose_secret(), "");
    assert!(stored.refresh_token.is_none());
    let held = vault.blob.as_ref().expect("the vault holds the secret");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(decoded.key.expect("a key").expose_secret(), "sk-secret");
}

#[tokio::test]
async fn save_stages_the_metadata_before_the_vault_takes_the_secret() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.stage = StageRefusal::Always;
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, record(2_000.0)).await;

    assert!(matches!(outcome, SecretSave::CredentialNotSaved(_)));
    assert!(vault.writes().is_empty());
    let kept = files.stored().expect("the previous login survives");
    assert_eq!(kept.timestamp.to_bits(), 1_000.0_f64.to_bits());
    assert_eq!(
        kept.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
}

#[tokio::test]
async fn save_reports_not_recorded_when_the_vault_took_the_secret_but_the_file_did_not() {
    let mut files = scripted_files();
    files.refusals.push(Refusal::Commit);
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, record(1_000.0)).await;

    assert!(matches!(outcome, SecretSave::CredentialNotRecorded(_)));
    assert!(vault.blob.is_some());
    assert!(files.stored().is_none());
}

#[rstest]
#[case::unreachable(KeyringUnusable::KeyringUnreachable, "keyring_unreachable")]
#[case::not_installed(KeyringUnusable::KeyringNotInstalled, "keyring_not_installed")]
#[case::disabled(KeyringUnusable::KeyringDisabled, "keyring_disabled")]
#[tokio::test]
async fn save_keeps_the_secret_in_the_file_when_the_keychain_is_unusable(
    #[case] failure: KeyringUnusable,
    #[case] expected: &str,
) {
    let files = scripted_files();
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);
    vault.failure = failure;

    let outcome = save_cli_token(&files, &mut vault, record(1_000.0)).await;

    let reported = match outcome {
        SecretSave::KeyringUnreachable => "keyring_unreachable",
        SecretSave::KeyringNotInstalled => "keyring_not_installed",
        SecretSave::KeyringDisabled => "keyring_disabled",
        SecretSave::SecretStored => "secret_stored",
        SecretSave::KeyringDiscardsWrites => "keyring_discards_writes",
        SecretSave::CredentialNotSaved(_) => "credential_not_saved",
        SecretSave::CredentialNotRecorded(_) => "credential_not_recorded",
    };
    assert_eq!(reported, expected);
    let stored = files.stored().expect("the owner-only file");
    assert_eq!(
        stored.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(
        stored
            .refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-secret"
    );
}

#[tokio::test]
async fn save_reports_not_saved_when_the_fallback_file_also_fails() {
    let mut files = scripted_files();
    files.refusals.push(Refusal::Write);
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let outcome = save_cli_token(&files, &mut vault, record(1_000.0)).await;

    assert!(matches!(outcome, SecretSave::CredentialNotSaved(_)));
    assert!(files.stored().is_none());
}

#[tokio::test]
async fn save_reports_a_keychain_that_discards_writes_and_falls_back_to_the_file() {
    let files = scripted_files();
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Discards);

    let outcome = save_cli_token(&files, &mut vault, record(1_000.0)).await;

    assert!(matches!(outcome, SecretSave::KeyringDiscardsWrites));
    let stored = files.stored().expect("the owner-only file");
    assert_eq!(
        stored.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
}

#[tokio::test]
async fn save_never_hands_a_record_without_a_secret_to_the_keychain() {
    let files = scripted_files();
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, metadata(1_000.0)).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    assert!(vault.writes().is_empty());
    assert!(files.stored().is_some());
}

#[tokio::test]
async fn save_stamps_past_the_file_login_on_a_clock_that_went_back() {
    let files = scripted_files();
    files.put(&record(2_000.0));
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, record(1_000.0)).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let expected = 2_000.0_f64.next_up();
    let held = vault.blob.as_ref().expect("the vault holds the secret");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(decoded.timestamp.to_bits(), expected.to_bits());
    let stored = files.stored().expect("the metadata file");
    assert_eq!(stored.timestamp.to_bits(), expected.to_bits());
}

#[tokio::test]
async fn save_stamps_past_the_keychain_login_the_file_could_not_record() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(encoded(&record(2_000.0)));
    vault.refusals.push(VaultRefusal::Unwritable);

    let outcome = save_cli_token(&files, &mut vault, record(1_500.0)).await;

    assert!(matches!(outcome, SecretSave::KeyringUnreachable));
    let expected = 2_000.0_f64.next_up();
    let stored = files.stored().expect("the owner-only file");
    assert_eq!(stored.timestamp.to_bits(), expected.to_bits());
}

#[tokio::test]
async fn save_stamps_past_a_login_stamped_at_the_same_instant() {
    let files = scripted_files();
    files.put(&metadata(2_000.0));
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, record(2_000.0)).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let expected = 2_000.0_f64.next_up();
    let held = vault.blob.as_ref().expect("the vault holds the secret");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(decoded.timestamp.to_bits(), expected.to_bits());
}

#[tokio::test]
async fn save_keeps_the_sign_in_time_on_a_clock_that_only_moves_forwards() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();

    let outcome = save_cli_token(&files, &mut vault, record(2_000.0)).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let held = vault.blob.as_ref().expect("the vault holds the secret");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(decoded.timestamp.to_bits(), 2_000.0_f64.to_bits());
    assert_eq!(
        files
            .stored()
            .expect("the metadata file")
            .timestamp
            .to_bits(),
        2_000.0_f64.to_bits()
    );
}

#[tokio::test]
async fn save_puts_a_jwt_only_credential_in_the_vault() {
    let files = scripted_files();
    let mut vault = fake_vault();
    let mut bare = metadata(1_000.0);
    bare.jwt_token = secret("jwt-only");

    let outcome = save_cli_token(&files, &mut vault, bare).await;

    assert!(matches!(outcome, SecretSave::SecretStored));
    let held = vault.blob.as_ref().expect("the vault holds the token");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(decoded.jwt_token.expose_secret(), "jwt-only");
    assert!(
        files
            .stored()
            .expect("the file")
            .jwt_token
            .expose_secret()
            .is_empty()
    );
}
