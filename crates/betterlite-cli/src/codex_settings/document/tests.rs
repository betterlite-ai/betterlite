#![cfg(test)]

use std::error::Error as _;
use std::fs;

use tempfile::TempDir;

use crate::codex_settings::document::read_settings;
use crate::codex_settings::error::CodexSettingsError;

#[tokio::test]
async fn read_settings_reports_a_directory() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::create_dir_all(&settings).unwrap();
    let error = read_settings(&settings).await.unwrap_err();
    assert!(matches!(error, CodexSettingsError::Read { .. }));
}

#[tokio::test]
async fn read_settings_reports_non_utf8() {
    let home = TempDir::new().unwrap();
    let settings = home.path().join("config.toml");
    fs::write(&settings, [0xff_u8, 0xfe, 0x00]).unwrap();
    let error = read_settings(&settings).await.unwrap_err();
    assert!(error.source().is_some());
    assert!(matches!(error, CodexSettingsError::NotUtf8 { .. }));
}
