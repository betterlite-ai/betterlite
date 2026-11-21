use betterlite_core::api::request_setup_options::callbacks::{
    CallbackSettings, merge_callback, validated_callback,
};
use betterlite_models::decisions::Json;
use betterlite_secrets::callback_vars::{Transform, transform_callback_vars};
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use crate::member_request_contracts::callbacks::TeamCallbackMetadata;
use crate::team_callback_policy::error::TeamCallbackError;
use crate::team_callback_policy::redaction::{
    CALLBACK_SETTINGS_KEY, LOGGING_KEY, mask_sensitive_callback_vars,
};

// _resolve_team_callbacks: what a request would really fire, so a logging slot that is present shadows callback_settings
pub fn resolve_team_callbacks<N, D>(
    team_metadata: &Json,
    cipher: &StoredCipher<N, D>,
) -> Result<TeamCallbackMetadata, TeamCallbackError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    if !matches!(team_metadata, Json::Object(_)) {
        return Ok(TeamCallbackMetadata::default());
    }
    let encoded = serde_json::to_value(team_metadata)
        .map_err(|source| TeamCallbackError::Metadata { source })?;
    let plain = transform_callback_vars(&encoded, &Transform::Decrypt(cipher));
    let mut decrypted: IndexMap<String, Json> =
        serde_json::from_value(plain).map_err(|source| TeamCallbackError::Metadata { source })?;
    let mut settings = match decrypted.shift_remove(LOGGING_KEY) {
        None | Some(Json::Null) => match decrypted.shift_remove(CALLBACK_SETTINGS_KEY) {
            Some(Json::Object(legacy)) => legacy,
            Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
            | None => IndexMap::new(),
        },
        Some(entries) => folded_settings(&entries),
    };
    // masking only rewrites text values, so a legacy slot LiteLLM's model rejects is still rejected below
    mask_sensitive_callback_vars(&mut settings);
    TeamCallbackMetadata::from_value(&Json::Object(settings))
        .map_err(|source| TeamCallbackError::CallbackSettings { source })
}

// a non-list slot folds nothing and a non-object entry is passed over without a warning
fn folded_settings(entries: &Json) -> IndexMap<String, Json> {
    let mut merged = CallbackSettings::default();
    let items = if let Json::List(items) = entries {
        items.as_slice()
    } else {
        &[]
    };
    for item in items.iter().filter(|item| matches!(item, Json::Object(_))) {
        match validated_callback(item) {
            Ok(callback) => merge_callback(&mut merged, &callback),
            Err(error) => {
                tracing::warn!("Ignoring invalid team-level read callback metadata: {error}");
            }
        }
    }
    let names = |list: Vec<String>| Json::List(list.into_iter().map(Json::Text).collect());
    let vars = merged
        .callback_vars
        .into_iter()
        .map(|(name, secret)| (name, Json::Text(secret.expose_secret().to_owned())))
        .collect();
    IndexMap::from([
        (
            "success_callback".to_owned(),
            names(merged.success_callback),
        ),
        (
            "failure_callback".to_owned(),
            names(merged.failure_callback),
        ),
        ("callbacks".to_owned(), names(merged.callbacks)),
        ("callback_vars".to_owned(), Json::Object(vars)),
    ])
}
