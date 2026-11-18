use betterlite_db::stored_cipher::DbCipher;
use betterlite_roi::settings::ROISettings;
use betterlite_roi::settings::ROISettingsUpdate;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::InternalError;
use super::error::RoiSettingsError;
use super::store::StoredSettings;

#[derive(Debug, Default, Deserialize)]
pub(super) struct PatchPresence {
    #[serde(default, with = "serde_with::rust::double_option")]
    pub(super) github_api_url: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub(super) github_token: Option<Option<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    pub(super) estimator_key: Option<Option<String>>,
}

pub(super) struct PatchedSecrets {
    pub(super) token: String,
    pub(super) encrypted_token: String,
    pub(super) estimator_key: String,
    pub(super) encrypted_estimator_key: String,
}

pub(super) fn patched_secrets(
    patch: &ROISettingsUpdate,
    presence: &PatchPresence,
    stored: &StoredSettings,
    current: &ROISettings,
    cipher: &DbCipher,
) -> Result<PatchedSecrets, RoiSettingsError> {
    let url = patch
        .github_api_url
        .as_deref()
        .unwrap_or(&current.github_api_url);
    let url_changed = url.trim_end_matches('/') != current.github_api_url.trim_end_matches('/');
    let (token, encrypted_token) = if presence.github_token.is_some() {
        let token = supplied_secret(patch.github_token.as_ref());
        let encrypted = encrypt(cipher, &token)?;
        (token, encrypted)
    } else if url_changed {
        (String::new(), String::new())
    } else {
        (
            current.github_token.expose_secret().to_owned(),
            stored.github_token.clone(),
        )
    };
    let estimator_key = if presence.estimator_key.is_some() {
        supplied_secret(patch.estimator_key.as_ref())
    } else {
        current.estimator_key.expose_secret().to_owned()
    };
    let encrypted_estimator_key = encrypt(cipher, &estimator_key)?;
    Ok(PatchedSecrets {
        token,
        encrypted_token,
        estimator_key,
        encrypted_estimator_key,
    })
}

fn supplied_secret(secret: Option<&SecretString>) -> String {
    let Some(value) = secret else {
        return String::new();
    };
    value.expose_secret().to_owned()
}

// LiteLLM encrypts only the non-empty secrets; an empty value stays an empty string
pub(super) fn encrypt(cipher: &DbCipher, value: &str) -> Result<String, RoiSettingsError> {
    if value.is_empty() {
        return Ok(String::new());
    }
    let plaintext = StoredValue::<()>::Text(SecretString::from(value.to_owned()));
    let sealed = cipher
        .encrypt_value(plaintext, None)
        .map_err(|_error| RoiSettingsError::Internal(InternalError::Encryption))?;
    match sealed {
        StoredValue::Text(text) => Ok(text.expose_secret().to_owned()),
        StoredValue::Other(()) => Ok(value.to_owned()),
    }
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
        github_token: SecretString::from(secrets.token.clone()),
        estimator_key: SecretString::from(secrets.estimator_key.clone()),
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

pub(super) fn stored_from(settings: &ROISettings, secrets: &PatchedSecrets) -> StoredSettings {
    StoredSettings {
        github_api_url: settings.github_api_url.clone(),
        github_token: secrets.encrypted_token.clone(),
        estimator_key: secrets.encrypted_estimator_key.clone(),
        repos: settings.repos.clone(),
        estimator_model: settings.estimator_model.clone(),
        estimator_prompt: settings.estimator_prompt.clone(),
        backfill_days: settings.backfill_days,
        update_interval_minutes: settings.update_interval_minutes,
        identity_map: settings.identity_map.clone(),
    }
}
