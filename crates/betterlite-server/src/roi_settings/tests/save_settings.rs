#![cfg(test)]

use indexmap::IndexMap;

use crate::roi_settings::error::InternalError;
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::store::StoredSettings;
use crate::roi_settings::store::save_settings;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::disconnected_repository;
use crate::roi_settings::support::sealed;
use crate::roi_settings::support::stored_settings;

#[tokio::test]
async fn save_settings_stores_the_encrypted_secrets_under_the_settings_key() {
    let repository = MemoryRepository::default();
    let mut stored = stored_settings();
    stored.github_token = sealed("ghp-token");
    stored.estimator_key = sealed("sk-estimator");
    save_settings(&repository, &stored).await.expect("saved");
    let saved = repository.stored().expect("stored settings");
    assert_eq!(saved, stored);
    assert_ne!(saved.github_token, "ghp-token");
    assert_ne!(saved.estimator_key, "sk-estimator");
}

#[tokio::test]
async fn save_settings_round_trips_every_field() {
    let repository = MemoryRepository::default();
    let mut identity_map = IndexMap::new();
    identity_map.insert("login".to_owned(), "user@example.com".to_owned());
    let stored = StoredSettings {
        github_api_url: "https://github.example.com".to_owned(),
        github_token: sealed("ghp-token"),
        estimator_key: sealed("sk-estimator"),
        repos: vec!["owner/repo".to_owned()],
        estimator_model: "gpt-4o".to_owned(),
        estimator_prompt: "Estimate the work.".to_owned(),
        backfill_days: 30,
        update_interval_minutes: 60.0,
        identity_map,
    };
    save_settings(&repository, &stored).await.expect("saved");
    assert_eq!(repository.stored().expect("stored settings"), stored);
}

#[tokio::test]
async fn save_settings_maps_a_repository_failure_to_internal() {
    assert!(matches!(
        save_settings(&disconnected_repository(), &StoredSettings::default()).await,
        Err(RoiSettingsError::Internal(InternalError::Config(_)))
    ));
}
