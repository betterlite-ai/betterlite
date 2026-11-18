use betterlite_db::budget_permission_rows::JsonData;
use betterlite_roi::settings::DEFAULT_PROMPT;
use betterlite_roi::settings::ROISettings;
use betterlite_roi::settings::SettingsError;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::DecryptPolicy;
use betterlite_secrets::secret_codec::FailureLog;
use betterlite_secrets::secret_codec::FailureValue;
use betterlite_secrets::secret_codec::NonceSource;
use betterlite_secrets::secret_codec::StoredCipher;
use betterlite_secrets::secret_codec::StoredValue;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::error::InternalError;
use super::error::RoiSettingsError;
use super::repository::SettingsRepository;

pub const SETTINGS_KEY: &str = "roi_calculator_settings";
pub const REPORT_KEY: &str = "roi_calculator_report";
pub(super) const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";

const DECRYPT: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

#[derive(Debug, Error)]
pub enum StoredSettingsError {
    #[error("the stored settings do not decode")]
    Decode(#[source] serde_json::Error),
    #[error("the stored settings are outside the allowed range")]
    Bounds,
    #[error("the stored settings do not validate")]
    Validate(#[source] SettingsError),
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredSettings {
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
            return Err(RoiSettingsError::StoredSettings(
                StoredSettingsError::Bounds,
            ));
        }
        let interval = self.update_interval_minutes;
        if !interval.is_finite() || !(0.0_f64..=43_200.0_f64).contains(&interval) {
            return Err(RoiSettingsError::StoredSettings(
                StoredSettingsError::Bounds,
            ));
        }
        Ok(())
    }
}

pub trait SettingsCipher: Send + Sync {
    fn seal(&self, value: &SecretString) -> Result<String, SecretError>;
    fn open(&self, value: &str) -> SecretString;
}

impl<N, D> SettingsCipher for StoredCipher<N, D>
where
    N: NonceSource + Send + Sync,
    D: SecretDiagnostics + Send + Sync,
{
    fn seal(&self, value: &SecretString) -> Result<String, SecretError> {
        // LiteLLM encrypts only the non-empty secrets; an empty value stays an empty string
        if value.expose_secret().is_empty() {
            return Ok(String::new());
        }
        let sealed = self.encrypt_value(StoredValue::<()>::Text(value.clone()), None)?;
        match sealed {
            StoredValue::Text(text) => Ok(text.expose_secret().to_owned()),
            StoredValue::Other(()) => Ok(value.expose_secret().to_owned()),
        }
    }

    // LiteLLM's decrypt_value_helper in _load_settings: a value that does not decrypt becomes an empty secret
    fn open(&self, value: &str) -> SecretString {
        if value.is_empty() {
            return SecretString::default();
        }
        let stored = StoredValue::<()>::Text(SecretString::from(value.to_owned()));
        match self.decrypt_value(stored, SETTINGS_KEY, DECRYPT) {
            Some(StoredValue::Text(plaintext)) => plaintext,
            Some(StoredValue::Other(())) | None => SecretString::default(),
        }
    }
}

pub async fn load_parameter<T, F>(
    repository: &dyn SettingsRepository,
    key: &str,
    invalid: F,
) -> Result<Option<T>, RoiSettingsError>
where
    T: DeserializeOwned,
    F: FnOnce(serde_json::Error) -> RoiSettingsError,
{
    let stored = repository
        .param(key)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))?;
    let Some(parameter) = stored else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(&parameter).map_err(InternalError::Encode)?;
    serde_json::from_slice(&bytes).map(Some).map_err(invalid)
}

pub async fn load_stored(
    repository: &dyn SettingsRepository,
) -> Result<StoredSettings, RoiSettingsError> {
    let invalid = |source| RoiSettingsError::StoredSettings(StoredSettingsError::Decode(source));
    let Some(stored) =
        load_parameter::<StoredSettings, _>(repository, SETTINGS_KEY, invalid).await?
    else {
        return Ok(StoredSettings::default());
    };
    stored.check_bounds()?;
    Ok(stored)
}

pub(super) fn open_settings(stored: &StoredSettings, cipher: &dyn SettingsCipher) -> ROISettings {
    ROISettings {
        github_api_url: stored.github_api_url.clone(),
        github_token: cipher.open(&stored.github_token),
        estimator_key: cipher.open(&stored.estimator_key),
        repos: stored.repos.clone(),
        estimator_model: stored.estimator_model.clone(),
        estimator_prompt: stored.estimator_prompt.clone(),
        backfill_days: stored.backfill_days,
        update_interval_minutes: stored.update_interval_minutes,
        identity_map: stored.identity_map.clone(),
    }
}

pub(super) fn validate_settings(settings: ROISettings) -> Result<ROISettings, RoiSettingsError> {
    settings
        .validate()
        .map_err(|source| RoiSettingsError::StoredSettings(StoredSettingsError::Validate(source)))
}

pub async fn load_settings(
    repository: &dyn SettingsRepository,
    cipher: &dyn SettingsCipher,
) -> Result<ROISettings, RoiSettingsError> {
    let stored = load_stored(repository).await?;
    let settings = open_settings(&stored, cipher);
    validate_settings(settings)
}

pub async fn save_settings(
    repository: &dyn SettingsRepository,
    stored: &StoredSettings,
) -> Result<(), RoiSettingsError> {
    let bytes = serde_json::to_vec(stored).map_err(InternalError::Encode)?;
    let value: JsonData = serde_json::from_slice(&bytes).map_err(InternalError::Encode)?;
    repository
        .set_param(SETTINGS_KEY, value)
        .await
        .map_err(|error| RoiSettingsError::Internal(InternalError::Config(error)))
}
