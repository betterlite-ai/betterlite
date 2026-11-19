use betterlite_db::budget_permission_rows::JsonData;
use betterlite_roi::settings::ROISettings;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::roi_settings::error::{InternalError, RoiSettingsError};
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::store::{SETTINGS_KEY, StoredSettingsError, load_parameter};

#[derive(Default, Deserialize)]
pub(super) struct Ciphertexts {
    #[serde(default)]
    pub(super) github_token: String,
    #[serde(default)]
    pub(super) estimator_key: String,
}

// LiteLLM's _StoredSettings writes every field on save; only the two ciphertexts are read back
#[derive(Serialize)]
struct StoredRecord<'settings> {
    github_api_url: &'settings str,
    github_token: &'settings str,
    estimator_key: &'settings str,
    repos: &'settings [String],
    estimator_model: &'settings str,
    estimator_prompt: &'settings str,
    backfill_days: i64,
    update_interval_minutes: f64,
    identity_map: &'settings IndexMap<String, String>,
}

pub(super) async fn load_ciphertexts(
    repository: &dyn SettingsRepository,
) -> Result<Ciphertexts, RoiSettingsError> {
    let invalid = |source| RoiSettingsError::StoredSettings(StoredSettingsError::Decode(source));
    let stored = load_parameter::<Ciphertexts, _>(repository, SETTINGS_KEY, invalid).await?;
    Ok(stored.unwrap_or_default())
}

pub(super) async fn save_settings(
    repository: &dyn SettingsRepository,
    settings: &ROISettings,
    stored: &Ciphertexts,
) -> Result<(), RoiSettingsError> {
    let record = StoredRecord {
        github_api_url: &settings.github_api_url,
        github_token: &stored.github_token,
        estimator_key: &stored.estimator_key,
        repos: &settings.repos,
        estimator_model: &settings.estimator_model,
        estimator_prompt: &settings.estimator_prompt,
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        identity_map: &settings.identity_map,
    };
    let bytes = serde_json::to_vec(&record).map_err(InternalError::Encode)?;
    let value: JsonData = serde_json::from_slice(&bytes).map_err(InternalError::Encode)?;
    repository
        .set_param(SETTINGS_KEY, value)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))
}
