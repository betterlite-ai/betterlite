use secrecy::ExposeSecret as _;

use crate::credential_lifecycle::load::load_cli_token;
use crate::token_records::secret::decode_secret;

use super::support::{
    OTHER_SERVER, Refusal, SERVER, StageRefusal, VaultRefusal, encoded, fake_vault, metadata,
    record, scripted_files, secret, vault_entry,
};

#[tokio::test]
async fn load_returns_none_without_a_token_file() {
    let files = scripted_files();
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(SERVER, "sk-vault", None, 2_000.0));

    assert!(load_cli_token(&files, &mut vault).await.is_none());
    assert_eq!(vault.reads(), 0);
}

#[tokio::test]
async fn load_takes_the_secret_from_the_vault_when_the_file_holds_only_metadata() {
    let files = scripted_files();
    files.put(&metadata(2_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(SERVER, "sk-vault", None, 2_000.0));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-vault"
    );
    assert_eq!(loaded.jwt_token.expose_secret(), "");
    assert!(loaded.refresh_token.is_none());
    assert!(vault.writes().is_empty());
}

#[tokio::test]
async fn load_migrates_a_plaintext_secret_and_scrubs_the_file() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(vault.writes().len(), 1);
    let moved = vault.blob.as_ref().expect("the vault holds the secret");
    let held = decode_secret(moved.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(held.key.expect("a key").expose_secret(), "sk-secret");
    let scrubbed = files.stored().expect("the metadata file stays");
    assert!(scrubbed.key.is_none());
    assert_eq!(scrubbed.jwt_token.expose_secret(), "");
    assert!(scrubbed.refresh_token.is_none());
    assert_eq!(scrubbed.user_id, "user-1");
}

#[tokio::test]
async fn load_rejoins_the_keychain_key_with_the_refresh_token_left_on_disk() {
    let files = scripted_files();
    let mut on_disk = metadata(2_000.0);
    on_disk.refresh_token = Some(secret("refresh-secret"));
    files.put(&on_disk);
    let mut vault = fake_vault();
    vault.blob = Some(encoded(&record(2_000.0)));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(
        loaded
            .refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-secret"
    );
    let scrubbed = files.stored().expect("the metadata file stays");
    assert!(scrubbed.refresh_token.is_none());
    let moved = vault.blob.as_ref().expect("the vault holds the credential");
    let held = decode_secret(moved.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(held.key.expect("a key").expose_secret(), "sk-secret");
    assert_eq!(
        held.refresh_token.expect("a refresh").expose_secret(),
        "refresh-secret"
    );
}

#[tokio::test]
async fn load_serves_the_vault_secret_when_the_file_copy_is_older() {
    let files = scripted_files();
    let mut old = record(1_000.0);
    old.key = Some(secret("sk-old"));
    old.refresh_token = Some(secret("refresh-old"));
    files.put(&old);
    let mut vault = fake_vault();
    vault.blob = Some(encoded(&record(2_000.0)));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(loaded.timestamp.to_bits(), 2_000.0_f64.to_bits());
    assert!(vault.writes().is_empty());
    let scrubbed = files.stored().expect("the metadata file stays");
    assert!(scrubbed.key.is_none());
    assert!(scrubbed.refresh_token.is_none());
}

#[tokio::test]
async fn load_serves_the_file_secret_when_the_vault_copy_is_older() {
    let files = scripted_files();
    let mut fresh = record(2_000.0);
    fresh.key = Some(secret("sk-fresh"));
    files.put(&fresh);
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(SERVER, "sk-stale", None, 1_000.0));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-fresh"
    );
    let moved = vault
        .blob
        .as_ref()
        .expect("the vault holds the fresh secret");
    let held = decode_secret(moved.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(held.key.expect("a key").expose_secret(), "sk-fresh");
    assert!(files.stored().expect("the file stays").key.is_none());
}

#[tokio::test]
async fn load_returns_none_for_a_metadata_only_file_and_an_empty_vault() {
    let files = scripted_files();
    files.put(&metadata(1_000.0));
    let mut vault = fake_vault();

    assert!(load_cli_token(&files, &mut vault).await.is_none());
}

#[tokio::test]
async fn load_returns_the_record_without_a_key_when_the_keychain_is_unreachable() {
    let files = scripted_files();
    files.put(&metadata(1_000.0));
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unavailable);

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert!(loaded.key.is_none());
    assert_eq!(loaded.user_id, "user-1");
    assert!(vault.writes().is_empty());
}

#[tokio::test]
async fn load_ignores_a_vault_secret_minted_for_another_server() {
    let files = scripted_files();
    files.put(&metadata(1_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(OTHER_SERVER, "sk-elsewhere", None, 1_000.0));

    assert!(load_cli_token(&files, &mut vault).await.is_none());
    assert!(vault.writes().is_empty());
    let kept = vault.blob.as_ref().expect("the vault keeps its entry");
    let held = decode_secret(kept.expose_secret(), OTHER_SERVER).expect("the vault entry pairs");
    assert_eq!(held.key.expect("a key").expose_secret(), "sk-elsewhere");
}

#[tokio::test]
async fn load_migrates_a_file_secret_past_a_vault_entry_for_another_server() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(vault_entry(OTHER_SERVER, "sk-elsewhere", None, 1_000.0));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    let moved = vault
        .blob
        .as_ref()
        .expect("the vault holds the file's secret");
    let held = decode_secret(moved.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(held.key.expect("a key").expose_secret(), "sk-secret");
}

#[tokio::test]
async fn load_keeps_the_file_secret_when_the_vault_refuses_the_write() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.refusals.push(VaultRefusal::Unwritable);

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    let kept = files.stored().expect("the file stays");
    assert_eq!(
        kept.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(
        kept.refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-secret"
    );
    assert_eq!(vault.writes().len(), 1);
}

#[tokio::test]
async fn load_finishes_the_scrub_in_place_when_the_commit_is_refused() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.refusals.push(Refusal::Commit);
    let mut vault = fake_vault();

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert!(vault.blob.is_some());
    let scrubbed = files.stored().expect("the file stays");
    assert!(scrubbed.key.is_none());
    assert!(scrubbed.refresh_token.is_none());
}

#[tokio::test]
async fn load_takes_the_vault_write_back_when_the_file_gives_up_its_secret() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.refusals.push(Refusal::Commit);
    files.refusals.push(Refusal::Overwrite);
    let mut vault = fake_vault();

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(vault.erases(), 1);
    assert!(vault.blob.is_none());
    let kept = files.stored().expect("the file still holds the secret");
    assert_eq!(
        kept.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
}

#[tokio::test]
async fn load_keeps_the_rejoined_entry_when_the_file_refuses_the_rewrite() {
    let mut files = scripted_files();
    let mut on_disk = metadata(2_000.0);
    on_disk.refresh_token = Some(secret("refresh-secret"));
    files.put(&on_disk);
    files.refusals.push(Refusal::Commit);
    files.refusals.push(Refusal::Overwrite);
    let mut vault = fake_vault();
    vault.blob = Some(encoded(&record(2_000.0)));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded
            .refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-secret"
    );
    assert_eq!(vault.erases(), 0);
    let held = vault.blob.as_ref().expect("the vault keeps both halves");
    let decoded = decode_secret(held.expose_secret(), SERVER).expect("the vault entry pairs");
    assert_eq!(
        decoded.refresh_token.expect("a refresh").expose_secret(),
        "refresh-secret"
    );
}

#[tokio::test]
async fn load_never_hands_the_vault_a_copy_it_could_not_scrub() {
    let mut files = scripted_files();
    files.put(&record(1_000.0));
    files.stage = StageRefusal::Always;
    let mut vault = fake_vault();

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert!(vault.writes().is_empty());
    let kept = files.stored().expect("the file stays");
    assert_eq!(
        kept.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
}

#[tokio::test]
async fn load_serves_the_file_secret_when_the_vault_holds_an_unreadable_blob() {
    let files = scripted_files();
    files.put(&record(1_000.0));
    let mut vault = fake_vault();
    vault.blob = Some(secret("not json at all"));

    let loaded = load_cli_token(&files, &mut vault).await.expect("loads");

    assert_eq!(
        loaded.key.as_ref().expect("a key").expose_secret(),
        "sk-secret"
    );
    assert_eq!(vault.writes().len(), 1);
}
