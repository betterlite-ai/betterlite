use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::database_faults::DbError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::StoredValue;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use super::CacheSettingsUpdateState;
use crate::audit_events::enums::AuditAction;
use crate::cache_settings::merge::merge_over_saved;
use crate::cache_settings::parse::parse_stored_settings;
use crate::cache_settings::precedence::resolve_cache_url_precedence;
use crate::cache_settings::redact::redact_credentials;
use crate::cache_settings::redact::redact_settings;
use crate::cache_settings_read::stored::StoredSettingsError;
use crate::cache_settings_read::stored::stored_from_raw;
use crate::cache_settings_read::view::carries_value;
use crate::cache_settings_update::audit::{CacheSettingsAuditEntry, spawn_cache_settings_audit};
use crate::cache_settings_update::manager::{CacheParams, CacheReinitError, REDIS_TYPE_FIELD};
use crate::cache_settings_update::store::CacheConfigRepository;

const SUCCESS_MESSAGE: &str = "Cache settings updated successfully";

#[derive(Debug, thiserror::Error)]
pub(super) enum UpdateError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Stored(#[from] StoredSettingsError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Reinit(#[from] CacheReinitError),
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct KeyView {
    user_id: Option<String>,
    api_key: Option<SecretString>,
}

#[derive(serde::Serialize)]
struct UpdateReply {
    message: &'static str,
    status: &'static str,
    settings: JsonValue,
}

struct StoredSettings {
    action: AuditAction,
    before: JsonMap,
    settings: JsonMap,
    params: CacheParams,
}

// LiteLLM's POST body: read the row, merge, encrypt, save, then decrypt what was saved for the reinitialisation.
pub(super) async fn update(
    state: &CacheSettingsUpdateState,
    repository: &dyn CacheConfigRepository,
    incoming: &JsonMap,
    litellm_changed_by: Option<&str>,
    principal: &Principal,
) -> Result<Response, UpdateError> {
    let stored = store_settings(state, repository, incoming).await?;
    state.reinit.init_cache(&stored.params)?;
    state.manager.update_params(stored.params);
    state.reinit.switch_on_response_caching();
    let entry = audit_entry(
        stored.action,
        litellm_changed_by,
        principal,
        &stored.before,
        &stored.settings,
    )?;
    spawn_cache_settings_audit(Arc::clone(&state.audit), state.spawner.as_ref(), entry);
    Ok(reply(&stored.settings))
}

async fn store_settings(
    state: &CacheSettingsUpdateState,
    repository: &dyn CacheConfigRepository,
    incoming: &JsonMap,
) -> Result<StoredSettings, UpdateError> {
    let stored = repository.stored_cache_settings().await?;
    let action = if stored.is_some() {
        AuditAction::Updated
    } else {
        AuditAction::Created
    };
    let raw = stored.unwrap_or(JsonValue::Null);
    let before = stored_before(&raw)?;
    let saved = stored_from_raw(&raw, &state.cipher)?;
    let settings = resolve_cache_url_precedence(&merge_over_saved(incoming, &saved));
    let encrypted = JsonValue::Object(encrypt_settings(&settings, &state.cipher)?);
    repository
        .upsert_cache_settings(&serde_json::to_string(&encrypted)?)
        .await?;
    let decrypted = stored_from_raw(&encrypted, &state.cipher)?;
    let mut params: CacheParams = serde_json::from_value(JsonValue::Object(decrypted))?;
    params.shift_remove(REDIS_TYPE_FIELD);
    Ok(StoredSettings {
        action,
        before,
        settings,
        params,
    })
}

// The audit keeps only the stored row's keys: the before_value never carries a plaintext or encrypted secret.
fn stored_before(raw: &JsonValue) -> Result<JsonMap, StoredSettingsError> {
    if !carries_value(raw) {
        return Ok(JsonMap::new());
    }
    Ok(parse_stored_settings(raw)?)
}

fn encrypt_settings(settings: &JsonMap, cipher: &DbCipher) -> Result<JsonMap, UpdateError> {
    let mut encrypted = settings.clone();
    for value in encrypted.values_mut() {
        let JsonValue::String(text) = value else {
            continue;
        };
        let encrypted_value = cipher.encrypt_value(
            StoredValue::<()>::Text(SecretString::from(text.clone())),
            None,
        )?;
        if let StoredValue::Text(sealed) = encrypted_value {
            *value = JsonValue::String(sealed.expose_secret().to_owned());
        }
    }
    Ok(encrypted)
}

fn audit_entry(
    action: AuditAction,
    litellm_changed_by: Option<&str>,
    principal: &Principal,
    before: &JsonMap,
    after: &JsonMap,
) -> Result<CacheSettingsAuditEntry, UpdateError> {
    let key = key_view(principal)?;
    Ok(CacheSettingsAuditEntry {
        action,
        litellm_changed_by: litellm_changed_by.map(str::to_owned),
        user_id: key.user_id,
        api_key: key.api_key,
        before_value: audit_text(before),
        updated_values: audit_text(after),
    })
}

fn key_view(principal: &Principal) -> Result<KeyView, serde_json::Error> {
    let Some(key) = principal.source_key.0.as_ref() else {
        return Ok(KeyView::default());
    };
    serde_json::from_value(serde_json::to_value(key)?)
}

fn audit_text(settings: &JsonMap) -> String {
    let redacted = JsonValue::Object(redact_settings(settings));
    format!("{{\"settings\":{redacted}}}")
}

fn reply(settings: &JsonMap) -> Response {
    let redacted = JsonValue::Object(redact_credentials(settings));
    (
        StatusCode::OK,
        Json(UpdateReply {
            message: SUCCESS_MESSAGE,
            status: "success",
            settings: redacted,
        }),
    )
        .into_response()
}
