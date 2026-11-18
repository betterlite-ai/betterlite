#![cfg(test)]

use betterlite_roi::settings::DEFAULT_PROMPT;
use rstest::rstest;

use crate::roi_settings::error::InternalError;
use crate::roi_settings::error::RoiSettingsError;
use crate::roi_settings::store::DEFAULT_GITHUB_API_URL;
use crate::roi_settings::store::SETTINGS_KEY;
use crate::roi_settings::store::load_stored;
use crate::roi_settings::support::MemoryRepository;
use crate::roi_settings::support::disconnected_repository;

#[tokio::test]
async fn load_stored_without_a_parameter_uses_the_defaults() {
    let stored = load_stored(&MemoryRepository::default())
        .await
        .expect("defaults");
    assert_eq!(stored.github_api_url, DEFAULT_GITHUB_API_URL);
    assert_eq!(stored.github_token, "");
    assert_eq!(stored.estimator_key, "");
    assert!(stored.repos.is_empty());
    assert_eq!(stored.estimator_model, "");
    assert_eq!(stored.estimator_prompt, DEFAULT_PROMPT);
    assert_eq!(stored.backfill_days, 7);
    assert_eq!(stored.update_interval_minutes, 1440.0);
    assert!(stored.identity_map.is_empty());
}

#[tokio::test]
async fn load_stored_ignores_unknown_fields() {
    let repository = MemoryRepository::default();
    repository.seed(
        SETTINGS_KEY,
        r#"{"github_api_url":"https://github.example.com","unknown":1}"#,
    );
    let stored = load_stored(&repository).await.expect("stored settings");
    assert_eq!(stored.github_api_url, "https://github.example.com");
}

#[rstest]
#[case::lowest_accepted(1, true)]
#[case::highest_accepted(3650, true)]
#[case::below_the_range(0, false)]
#[case::above_the_range(3651, false)]
#[tokio::test]
async fn load_stored_checks_backfill_days(#[case] days: i64, #[case] accepted: bool) {
    let repository = MemoryRepository::default();
    repository.seed(SETTINGS_KEY, &format!(r#"{{"backfill_days":{days}}}"#));
    assert_eq!(load_stored(&repository).await.is_ok(), accepted);
}

#[rstest]
#[case::zero_is_valid(0.0, true)]
#[case::highest_accepted(43_200.0, true)]
#[case::negative(-1.0, false)]
#[case::above_the_range(43_200.5, false)]
#[tokio::test]
async fn load_stored_checks_update_interval_minutes(#[case] minutes: f64, #[case] accepted: bool) {
    let repository = MemoryRepository::default();
    repository.seed(
        SETTINGS_KEY,
        &format!(r#"{{"update_interval_minutes":{minutes}}}"#),
    );
    assert_eq!(load_stored(&repository).await.is_ok(), accepted);
}

#[tokio::test]
async fn load_stored_rejects_a_value_that_is_not_an_object() {
    let repository = MemoryRepository::default();
    repository.seed(SETTINGS_KEY, r#""not settings""#);
    assert!(matches!(
        load_stored(&repository).await,
        Err(RoiSettingsError::StoredSettings)
    ));
}

#[tokio::test]
async fn load_stored_maps_a_repository_failure_to_internal() {
    assert!(matches!(
        load_stored(&disconnected_repository()).await,
        Err(RoiSettingsError::Internal(InternalError::Config(_)))
    ));
}
