#![cfg(test)]

use std::fs;

use betterlite_logging::private_json::files::PrivateFiles;
use tempfile::TempDir;

use crate::codex_settings::configure::{ConfigureCodexSettings, configure_codex_settings};
use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::paths::{codex_configure_state_path, resolve_settings_path};
use crate::codex_settings::tests::support::{
    GATEWAY, KEY, MODEL, configure_with, document_at, item_at, journal_files, secret,
    staging_files, string_at, unconfigure_at,
};
use crate::codex_settings::version::CodexVersion;

#[cfg(unix)]
use crate::codex_settings::tests::support::{directory_mode, file_mode};

#[tokio::test]
async fn configure_keeps_an_inline_providers_table() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "model_providers = { other = { name = \"Keep\" } }\n",
    )
    .unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let configured = document_at(&settings);
    assert_eq!(
        string_at(&configured, "model_providers.other.name").as_deref(),
        Some("Keep")
    );
    assert_eq!(
        string_at(&configured, "model_providers.litellm.base_url").as_deref(),
        Some("https://gateway.example.com/team/v1")
    );
    unconfigure_at(&settings, &files).await.unwrap();
    let restored = document_at(&settings);
    assert_eq!(
        string_at(&restored, "model_providers.other.name").as_deref(),
        Some("Keep")
    );
    assert!(item_at(&restored, "model_providers.litellm").is_none());
}

#[tokio::test]
async fn configure_rejects_an_old_codex() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"keep\"\n").unwrap();
    let key = secret(KEY);
    let request = ConfigureCodexSettings {
        base_url: GATEWAY,
        api_key: &key,
        model: MODEL,
        settings_path: &settings,
        version: CodexVersion::Reported(Some("codex-cli 0.128.9")),
    };
    let error = configure_codex_settings(&request, &PrivateFiles::default())
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::UnsafeCodex));
    assert_eq!(fs::read_to_string(&settings).unwrap(), "model = \"keep\"\n");
    assert!(!codex_configure_state_path(&settings).unwrap().exists());
}

#[cfg(unix)]
#[tokio::test]
async fn configure_stages_owner_only_files() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    assert_eq!(directory_mode(state_path.parent().unwrap()), 0o700);
    assert_eq!(file_mode(&settings), 0o600);
    assert_eq!(file_mode(&state_path), 0o600);
}

#[tokio::test]
async fn repeat_configure_keeps_the_original_baseline() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let edited = fs::read_to_string(&settings)
        .unwrap()
        .replace(&format!("model = \"{MODEL}\""), "model = \"user-edited\"");
    fs::write(&settings, edited).unwrap();
    configure_with(&settings, &files, GATEWAY, "sk-rotated", "third-model")
        .await
        .unwrap();
    unconfigure_at(&settings, &files).await.unwrap();
    let document = document_at(&settings);
    assert_eq!(
        string_at(&document, "model").as_deref(),
        Some("user-edited")
    );
}

#[tokio::test]
async fn repeated_configure_keeps_the_displaced_value() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    configure_with(&settings, &files, GATEWAY, KEY, "second-model")
        .await
        .unwrap();
    unconfigure_at(&settings, &files).await.unwrap();
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("original"));
}

#[tokio::test]
async fn configure_round_trips_an_api_key_with_toml_specials() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let api_key = "sk-\"quoted\"\\slash\nnext-line";
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, api_key, MODEL)
        .await
        .unwrap();
    let document = document_at(&settings);
    assert_eq!(
        string_at(
            &document,
            "model_providers.litellm.http_headers.Authorization"
        ),
        Some(format!("Bearer {api_key}"))
    );
    unconfigure_at(&settings, &files).await.unwrap();
    assert!(!settings.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn configure_through_a_symlink_updates_the_target() {
    use std::os::unix::fs::symlink;

    let home = TempDir::new().unwrap();
    let target = home.path().join("real-config.toml");
    fs::write(&target, "model = \"old\"\n").unwrap();
    let alias = home.path().join("config.toml");
    symlink(&target, &alias).unwrap();
    let files = PrivateFiles::default();
    configure_with(&alias, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    assert!(alias.is_symlink());
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("Bearer sk-test")
    );
    assert_eq!(
        codex_configure_state_path(&alias).unwrap(),
        codex_configure_state_path(&target).unwrap()
    );
    unconfigure_at(&alias, &files).await.unwrap();
    assert!(alias.is_symlink());
    let document = document_at(&alias);
    assert_eq!(string_at(&document, "model").as_deref(), Some("old"));
}

#[cfg(unix)]
#[tokio::test]
async fn configure_refuses_a_non_utf8_settings_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let home = TempDir::new().unwrap();
    let settings = home
        .path()
        .join(OsString::from_vec(b"config-\xff.toml".to_vec()));
    let error = configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::NonUtf8Path { .. }));
    assert!(!settings.exists());
    assert!(!codex_configure_state_path(&settings).unwrap().exists());
}

#[tokio::test]
async fn configure_receipt_failure_discards_both_staged_files() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    let (files, journal) = journal_files();
    let state_path = codex_configure_state_path(&settings).unwrap();
    journal.fail_replace(&state_path, 1);
    let error = configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::WriteReceipt { .. }));
    assert_eq!(fs::read_to_string(&settings).unwrap(), "model = \"old\"\n");
    assert!(!state_path.exists());
    let resolved_parent = resolve_settings_path(&settings).unwrap();
    let parent = resolved_parent.parent().unwrap();
    let state_directory = parent.join(".litellm");
    assert!(staging_files(&[parent, state_directory.as_path()]).is_empty());
}
