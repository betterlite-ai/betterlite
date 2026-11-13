#![cfg(test)]

use std::fs;

use betterlite_logging::private_json::files::PrivateFiles;
use tempfile::TempDir;

use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::paths::{codex_configure_state_path, resolve_settings_path};
use crate::codex_settings::tests::support::{
    GATEWAY, KEY, MODEL, configure_with, document_at, journal_files, string_at, unconfigure_at,
};

#[tokio::test]
async fn unconfigure_without_receipt_reports_nothing_to_undo() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let error = unconfigure_at(&settings, &PrivateFiles::default())
        .await
        .unwrap_err();
    assert!(matches!(error, CodexSettingsError::NothingToUndo));
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        "model = \"original\"\n"
    );
}

#[tokio::test]
async fn unconfigure_keeps_an_edited_provider() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(
        &settings,
        "[model_providers.litellm]\nbase_url = \"https://old.example.com/v1\"\nhttp_headers = { Authorization = \"Bearer old-key\" }\n",
    )
    .unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let edited = fs::read_to_string(&settings)
        .unwrap()
        .replace(GATEWAY, "https://user.example.com");
    fs::write(&settings, edited).unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert_eq!(outcome.kept, vec!["model_providers.litellm".to_owned()]);
    let document = document_at(&settings);
    assert_eq!(
        string_at(&document, "model_providers.litellm.base_url").as_deref(),
        Some("https://user.example.com/v1")
    );
    assert_eq!(
        string_at(
            &document,
            "model_providers.litellm.http_headers.Authorization"
        )
        .as_deref(),
        Some("Bearer sk-test")
    );
    assert!(!fs::read_to_string(&settings).unwrap().contains("old-key"));
}

#[tokio::test]
async fn unconfigure_restores_a_dotted_provider() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model_providers.litellm.name = \"old\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    assert!(
        fs::read_to_string(&settings)
            .unwrap()
            .contains("Bearer sk-test")
    );
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert!(
        outcome
            .restored
            .contains(&"model_providers.litellm".to_owned())
    );
    let text = fs::read_to_string(&settings).unwrap();
    assert!(!text.contains("Authorization"));
    assert!(!text.contains("sk-test"));
    assert_eq!(
        string_at(&document_at(&settings), "model_providers.litellm.name").as_deref(),
        Some("old")
    );
}

#[tokio::test]
async fn unconfigure_skips_a_field_restored_unchanged() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    configure_with(&settings, &files, GATEWAY, KEY, "original")
        .await
        .unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert!(!outcome.restored.contains(&"model".to_owned()));
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("original"));
}

#[tokio::test]
async fn unconfigure_after_the_user_deleted_the_file() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\nprofile = \"work\"\n").unwrap();
    let files = PrivateFiles::default();
    configure_with(&settings, &files, GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    fs::remove_file(&settings).unwrap();
    let outcome = unconfigure_at(&settings, &files).await.unwrap();
    assert!(outcome.file_removed);
    assert!(outcome.restored.is_empty());
    assert!(!settings.exists());
}

#[tokio::test]
async fn unconfigure_keeps_the_receipt_when_the_write_fails() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"old\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let before = fs::read(&settings).unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    let (files, journal) = journal_files();
    journal.fail_replace(&resolve_settings_path(&settings).unwrap(), 1);
    let error = unconfigure_at(&settings, &files).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::Undo { .. }));
    assert_eq!(fs::read(&settings).unwrap(), before);
    assert!(state_path.exists());
    unconfigure_at(&settings, &PrivateFiles::default())
        .await
        .unwrap();
    let document = document_at(&settings);
    assert_eq!(string_at(&document, "model").as_deref(), Some("old"));
}
