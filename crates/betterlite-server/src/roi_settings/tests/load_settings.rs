#![cfg(test)]

use secrecy::ExposeSecret as _;

use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::store::DEFAULT_GITHUB_API_URL;
use crate::roi_settings::store::SETTINGS_KEY;
use crate::roi_settings::store::load_settings;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::cipher;
use crate::roi_settings::support::sealed;

const UNDECRYPTABLE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[tokio::test]
async fn load_settings_decrypts_both_secrets() {
    let repository = MemoryRepository::default();
    repository.seed(
        SETTINGS_KEY,
        &format!(
            r#"{{"github_token":"{}","estimator_key":"{}"}}"#,
            sealed("ghp-token"),
            sealed("sk-estimator"),
        ),
    );
    let settings = load_settings(&repository, &cipher())
        .await
        .expect("loaded settings");
    assert_eq!(settings.github_token.expose_secret(), "ghp-token");
    assert_eq!(settings.estimator_key.expose_secret(), "sk-estimator");
}

#[tokio::test]
async fn load_settings_uses_defaults_without_a_stored_parameter() {
    let settings = load_settings(&MemoryRepository::default(), &cipher())
        .await
        .expect("loaded defaults");
    assert_eq!(settings.github_api_url, DEFAULT_GITHUB_API_URL);
    assert_eq!(settings.github_token.expose_secret(), "");
    assert_eq!(settings.estimator_key.expose_secret(), "");
}

#[tokio::test]
async fn load_settings_treats_an_undecryptable_secret_as_empty() {
    let repository = MemoryRepository::default();
    repository.seed(
        SETTINGS_KEY,
        &format!(r#"{{"github_token":"{UNDECRYPTABLE}","estimator_key":"{UNDECRYPTABLE}"}}"#),
    );
    let settings = load_settings(&repository, &cipher())
        .await
        .expect("loaded settings");
    assert_eq!(settings.github_token.expose_secret(), "");
    assert_eq!(settings.estimator_key.expose_secret(), "");
}

#[tokio::test]
async fn load_settings_rejects_settings_that_fail_validation() {
    let repository = MemoryRepository::default();
    repository.seed(SETTINGS_KEY, r#"{"repos":["not-a-repository"]}"#);
    assert!(matches!(
        load_settings(&repository, &cipher()).await,
        Err(RoiSettingsError::StoredSettings(_))
    ));
}

#[tokio::test]
async fn load_settings_rejects_an_interval_between_zero_and_five_minutes() {
    let repository = MemoryRepository::default();
    repository.seed(SETTINGS_KEY, r#"{"update_interval_minutes":3}"#);
    assert!(matches!(
        load_settings(&repository, &cipher()).await,
        Err(RoiSettingsError::StoredSettings(_))
    ));
}
