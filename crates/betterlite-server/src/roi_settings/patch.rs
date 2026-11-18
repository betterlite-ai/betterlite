use betterlite_roi::settings::ROISettings;
use betterlite_roi::settings::ROISettingsUpdate;
use betterlite_secrets::error::SecretError;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::InternalError;
use super::error::RoiSettingsError;
use super::store::SettingsCipher;
use super::store::StoredSettings;

impl From<SecretError> for RoiSettingsError {
    fn from(source: SecretError) -> Self {
        Self::Internal(InternalError::Encryption(source))
    }
}

#[derive(Deserialize)]
#[expect(
    clippy::option_option,
    reason = "a present key can carry a null value that clears the stored secret"
)]
pub struct PatchPresence {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub(super) github_api_url: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub(super) github_token: Option<Option<SecretString>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub(super) estimator_key: Option<Option<SecretString>>,
}

pub(super) struct PatchedSecrets {
    pub(super) token: SecretString,
    pub(super) encrypted_token: String,
    pub(super) estimator_key: SecretString,
    pub(super) encrypted_estimator_key: String,
}

pub(super) fn patched_secrets(
    patch: &ROISettingsUpdate,
    presence: &PatchPresence,
    stored: &StoredSettings,
    current: &ROISettings,
    cipher: &dyn SettingsCipher,
) -> Result<PatchedSecrets, RoiSettingsError> {
    let url = patch
        .github_api_url
        .as_deref()
        .unwrap_or(&current.github_api_url);
    let url_changed = url.trim_end_matches('/') != current.github_api_url.trim_end_matches('/');
    let (token, encrypted_token) = if presence.github_token.is_some() {
        let token = patch.github_token.clone().unwrap_or_default();
        let encrypted = cipher.seal(&token)?;
        (token, encrypted)
    } else if url_changed {
        (SecretString::default(), String::new())
    } else {
        (current.github_token.clone(), stored.github_token.clone())
    };
    let estimator_key = if presence.estimator_key.is_some() {
        patch.estimator_key.clone().unwrap_or_default()
    } else {
        current.estimator_key.clone()
    };
    let encrypted_estimator_key = cipher.seal(&estimator_key)?;
    Ok(PatchedSecrets {
        token,
        encrypted_token,
        estimator_key,
        encrypted_estimator_key,
    })
}

pub(super) fn build_settings(
    patch: ROISettingsUpdate,
    current: &ROISettings,
    secrets: &PatchedSecrets,
) -> Result<ROISettings, RoiSettingsError> {
    let settings = ROISettings {
        github_api_url: patch
            .github_api_url
            .unwrap_or_else(|| current.github_api_url.clone()),
        github_token: secrets.token.clone(),
        estimator_key: secrets.estimator_key.clone(),
        update_interval_minutes: patch
            .update_interval_minutes
            .unwrap_or(current.update_interval_minutes),
        repos: patch.repos.unwrap_or_else(|| current.repos.clone()),
        estimator_model: patch
            .estimator_model
            .unwrap_or_else(|| current.estimator_model.clone()),
        estimator_prompt: patch
            .estimator_prompt
            .unwrap_or_else(|| current.estimator_prompt.clone()),
        backfill_days: patch.backfill_days.unwrap_or(current.backfill_days),
        identity_map: current.identity_map.clone(),
    };
    settings.validate().map_err(RoiSettingsError::Validation)
}
