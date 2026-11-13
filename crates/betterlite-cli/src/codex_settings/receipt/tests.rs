#![cfg(test)]

use std::fs;

use betterlite_logging::private_json::files::PrivateFiles;
use tempfile::TempDir;

use crate::codex_settings::OWNED_PATHS;
use crate::codex_settings::error::CodexSettingsError;
use crate::codex_settings::paths::{
    codex_configure_state_path, resolve_settings_path, settings_path_text,
};
use crate::codex_settings::receipt::{CodexReceipt, ReceiptSource, read_receipt};
use crate::codex_settings::snapshot::{Snapshot, fingerprint};
use crate::codex_settings::tests::support::{GATEWAY, KEY, MODEL, configure_with};

#[tokio::test]
async fn receipt_records_missing_profile_as_the_sentinel() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\nprofile = \"work\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let receipt = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(
        receipt.written.get("profile").map(String::as_str),
        Some("ffa63583dfa6706b87d284b86b0d693a161e4840aad2c5cf6b5d27c3b9621f7d")
    );
    let saved = receipt
        .previous
        .get("profile")
        .and_then(|saved| saved.as_deref())
        .unwrap();
    assert_eq!(
        fingerprint(Some(&Snapshot::parse(saved).unwrap())),
        fingerprint(Some(&Snapshot::parse("value = \"work\"\n").unwrap()))
    );
    let recorded: Vec<&str> = OWNED_PATHS
        .iter()
        .copied()
        .filter(|path| receipt.written.contains_key(*path))
        .collect();
    assert_eq!(
        recorded,
        vec![
            "model_provider",
            "model",
            "profile",
            "model_providers.litellm"
        ]
    );
}

#[tokio::test]
async fn receipt_rejects_a_wrong_version() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let receipt = format!(
        "{{\"version\":2,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}}}}"
    );
    fs::write(&state_path, receipt).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Version(2),
            ..
        }
    ));
}

#[tokio::test]
async fn receipt_rejects_another_settings_path() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let other = home.path().join("other.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let quoted = serde_json::to_string(
        &settings_path_text(&resolve_settings_path(&other).unwrap()).unwrap(),
    )
    .unwrap();
    let receipt = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}}}}"
    );
    fs::write(&state_path, receipt).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Scope,
            ..
        }
    ));
}

#[tokio::test]
async fn receipt_rejects_a_mismatched_field_set() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let receipt = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{\"model\":null}},\"written\":{{}}}}"
    );
    fs::write(&state_path, receipt).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Keys,
            ..
        }
    ));
    let ownership = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{\"other\":null}},\"written\":{{\"other\":\"{}\"}}}}",
        "0".repeat(64)
    );
    fs::write(&state_path, ownership).unwrap();
    let ownership_error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        ownership_error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Ownership,
            ..
        }
    ));
}

#[tokio::test]
async fn receipt_rejects_unknown_fields() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let extra = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}},\"surprise\":true}}"
    );
    fs::write(&state_path, extra).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Json(_),
            ..
        }
    ));
}

#[tokio::test]
async fn receipt_round_trips_through_configure() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"original\"\nprofile = \"work\"\n").unwrap();
    configure_with(&settings, &PrivateFiles::default(), GATEWAY, KEY, MODEL)
        .await
        .unwrap();
    let receipt: CodexReceipt = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(receipt.version, 1);
    assert_eq!(receipt.previous.len(), receipt.written.len());
    assert!(receipt.file_existed);
    assert!(!receipt.providers_existed);
    let saved = receipt
        .previous
        .get("profile")
        .and_then(|saved| saved.as_deref())
        .unwrap();
    assert_eq!(
        fingerprint(Some(&Snapshot::parse(saved).unwrap())),
        fingerprint(Some(&Snapshot::parse("value = \"work\"\n").unwrap()))
    );
}

#[tokio::test]
async fn read_receipt_reports_a_directory() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, "model = \"keep\"\n").unwrap();
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(&state_path).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::Receipt { .. }));
}

#[tokio::test]
async fn receipt_accepts_a_missing_version() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let receipt = format!(
        "{{\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{}},\"written\":{{}}}}"
    );
    fs::write(&state_path, receipt).unwrap();
    let parsed = read_receipt(&settings).await.unwrap().unwrap();
    assert_eq!(parsed.version, 1);
}

#[tokio::test]
async fn receipt_rejects_a_multi_key_snapshot() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    let state_path = codex_configure_state_path(&settings).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    let resolved = resolve_settings_path(&settings).unwrap();
    let quoted = serde_json::to_string(&settings_path_text(&resolved).unwrap()).unwrap();
    let saved = serde_json::to_string("value = 1\nother = 2\n").unwrap();
    let receipt = format!(
        "{{\"version\":1,\"settings_path\":{quoted},\"file_existed\":false,\"providers_existed\":false,\"previous\":{{\"model\":{saved}}},\"written\":{{\"model\":\"{}\"}}}}",
        "0".repeat(64)
    );
    fs::write(&state_path, receipt).unwrap();
    let error = read_receipt(&settings).await.unwrap_err();
    assert!(matches!(
        error,
        CodexSettingsError::Receipt {
            source: ReceiptSource::Snapshot(_),
            ..
        }
    ));
}
