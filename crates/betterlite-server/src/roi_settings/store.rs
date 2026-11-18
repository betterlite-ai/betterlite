use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_roi::records::ROIReport;
use betterlite_roi::settings::DEFAULT_PROMPT;
use betterlite_roi::settings::ROISettings;
use betterlite_secrets::secret_codec::DecryptPolicy;
use betterlite_secrets::secret_codec::FailureLog;
use betterlite_secrets::secret_codec::FailureValue;
use betterlite_secrets::secret_codec::StoredValue;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;

use super::error::InternalError;
use super::error::RoiSettingsError;
use super::repository::SettingsRepository;

pub(super) const SETTINGS_KEY: &str = "roi_calculator_settings";
pub(super) const REPORT_KEY: &str = "roi_calculator_report";
pub(super) const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";

const DECRYPT: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct StoredSettings {
    pub(super) github_api_url: String,
    pub(super) github_token: String,
    pub(super) estimator_key: String,
    pub(super) repos: Vec<String>,
    pub(super) estimator_model: String,
    pub(super) estimator_prompt: String,
    pub(super) backfill_days: i64,
    pub(super) update_interval_minutes: f64,
    pub(super) identity_map: IndexMap<String, String>,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            github_api_url: DEFAULT_GITHUB_API_URL.to_owned(),
            github_token: String::new(),
            estimator_key: String::new(),
            repos: Vec::new(),
            estimator_model: String::new(),
            estimator_prompt: DEFAULT_PROMPT.to_owned(),
            backfill_days: 7,
            update_interval_minutes: 1440.0,
            identity_map: IndexMap::new(),
        }
    }
}

impl StoredSettings {
    fn check_bounds(&self) -> Result<(), RoiSettingsError> {
        if !(1..=3650).contains(&self.backfill_days) {
            return Err(RoiSettingsError::StoredSettings);
        }
        let interval = self.update_interval_minutes;
        if !interval.is_finite() || !(0.0..=43_200.0).contains(&interval) {
            return Err(RoiSettingsError::StoredSettings);
        }
        Ok(())
    }
}

// LiteLLM's decrypt_value_helper in _load_settings: a value that does not decrypt becomes an empty secret
pub(super) fn decrypt(cipher: &DbCipher, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let stored = StoredValue::<()>::Text(SecretString::from(value.to_owned()));
    match cipher.decrypt_value(stored, SETTINGS_KEY, DECRYPT) {
        Some(StoredValue::Text(plaintext)) => plaintext.expose_secret().to_owned(),
        Some(StoredValue::Other(())) | None => String::new(),
    }
}

pub(super) async fn load_stored(
    repository: &dyn SettingsRepository,
) -> Result<StoredSettings, RoiSettingsError> {
    let Some(parameter) = repository
        .param(SETTINGS_KEY)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))?
    else {
        return Ok(StoredSettings::default());
    };
    let bytes = serde_json::to_vec(&parameter)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    let stored: StoredSettings =
        serde_json::from_slice(&bytes).map_err(|_error| RoiSettingsError::StoredSettings)?;
    stored.check_bounds()?;
    Ok(stored)
}

pub(super) async fn load_settings(
    repository: &dyn SettingsRepository,
    cipher: &DbCipher,
) -> Result<ROISettings, RoiSettingsError> {
    let stored = load_stored(repository).await?;
    let settings = ROISettings {
        github_api_url: stored.github_api_url,
        github_token: SecretString::from(decrypt(cipher, &stored.github_token)),
        estimator_key: SecretString::from(decrypt(cipher, &stored.estimator_key)),
        repos: stored.repos,
        estimator_model: stored.estimator_model,
        estimator_prompt: stored.estimator_prompt,
        backfill_days: stored.backfill_days,
        update_interval_minutes: stored.update_interval_minutes,
        identity_map: stored.identity_map,
    };
    settings
        .validate()
        .map_err(|_error| RoiSettingsError::StoredSettings)
}

pub(super) async fn save_settings(
    repository: &dyn SettingsRepository,
    stored: &StoredSettings,
) -> Result<(), RoiSettingsError> {
    let bytes = serde_json::to_vec(stored)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    let value: JsonData = serde_json::from_slice(&bytes)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    repository
        .set_param(SETTINGS_KEY, value)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))
}

pub(super) async fn load_report(
    repository: &dyn SettingsRepository,
) -> Result<Option<ROIReport>, RoiSettingsError> {
    let Some(parameter) = repository
        .param(REPORT_KEY)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))?
    else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&parameter)
        .map_err(InternalError::Encode)
        .map_err(RoiSettingsError::Internal)?;
    let report = serde_json::from_slice(&bytes).map_err(|_error| RoiSettingsError::StoredReport)?;
    Ok(Some(report))
}
