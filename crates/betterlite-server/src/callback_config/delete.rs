use std::collections::BTreeMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::audit_events::enums::{AuditAction, AuditTable};
use crate::audit_events::identity::CallerIdentity;
use crate::auth::auth_identity::AuthContext;
use crate::config_audit::{ConfigChange, config_audit_log};
use crate::plugin_routes::routes::{Identity, PROXY_ADMIN_ROLE, caller_identity};
use crate::tracing_routes::router::unauthenticated;

use super::HandlerState;
use super::body;
use super::callbacks::{self, LITELLM_SETTINGS, SUCCESS_CALLBACK, SUCCESS_PATH};
use super::error::CallbackConfigError;

#[derive(Serialize)]
pub(crate) struct DeleteReply {
    message: String,
    removed_callback: String,
    remaining_callbacks: Vec<String>,
    deleted_at: String,
}

pub(crate) async fn delete_callback(
    State(state): State<HandlerState>,
    caller: Option<Extension<AuthContext>>,
    payload: Bytes,
) -> Response {
    let Some(Extension(context)) = caller else {
        return unauthenticated();
    };
    let request = match body::callback_delete_request(&payload) {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    let identity = match caller_identity(&context.principal) {
        Ok(identity) => identity,
        Err(error) => {
            return CallbackConfigError::Failure {
                message: error.to_string(),
            }
            .into_delete_response();
        }
    };
    let audit_identity = match audit_caller(&identity, &context.principal) {
        Ok(audit_identity) => audit_identity,
        Err(error) => {
            return CallbackConfigError::Failure {
                message: error.to_string(),
            }
            .into_delete_response();
        }
    };
    match remove_callback(&state, &identity, &audit_identity, &request.callback_name).await {
        Ok(reply) => Json(reply).into_response(),
        Err(error) => error.into_delete_response(),
    }
}

// The delete's gates run in LiteLLM's order, before the config is read: database, role, store_model_in_db.
fn check_gates(state: &HandlerState, identity: &Identity) -> Result<(), CallbackConfigError> {
    if !state.config.database_connected {
        return Err(CallbackConfigError::DatabaseNotConnected);
    }
    if identity.user_role.as_deref() != Some(PROXY_ADMIN_ROLE) {
        return Err(CallbackConfigError::NotAllowedAccess {
            role: identity
                .user_role
                .clone()
                .unwrap_or_else(|| "None".to_owned()),
        });
    }
    if !state.config.store_model_in_db.store_model_in_db() {
        return Err(CallbackConfigError::StorageDisabled);
    }
    Ok(())
}

async fn remove_callback(
    state: &HandlerState,
    identity: &Identity,
    audit_identity: &CallerIdentity,
    requested: &str,
) -> Result<DeleteReply, CallbackConfigError> {
    check_gates(state, identity)?;
    let mut config = state.config.overlay.reload_config().await?;
    let mut settings =
        callbacks::section_object(config.values.get(LITELLM_SETTINGS), LITELLM_SETTINGS)?;
    let mut success = callbacks::strict_entries(settings.get(SUCCESS_CALLBACK), SUCCESS_PATH)?;
    let callback_name = requested.to_lowercase();
    // the lower-cased name is compared against the stored names as they are, never lower-cased themselves
    let Some(index) = success.iter().position(|entry| *entry == callback_name) else {
        return Err(CallbackConfigError::CallbackNotFound {
            name: callback_name,
        });
    };
    let before = callback_change(&success);
    success.remove(index);
    let remaining = success.clone();
    let after = callback_change(&remaining);
    settings.insert(SUCCESS_CALLBACK.to_owned(), callback_list(&success));
    config.values.insert(
        LITELLM_SETTINGS.to_owned(),
        JsonValue::Object(settings.into_iter().collect()),
    );
    state.config.saver.save_config(config).await?;
    record_deletion(state, audit_identity, &before, &after).await;
    state.deployments.add_deployment().await;
    Ok(DeleteReply {
        message: format!("Successfully deleted callback: {callback_name}"),
        removed_callback: callback_name,
        remaining_callbacks: remaining,
        deleted_at: local_timestamp(&state.config.clock.now()),
    })
}

async fn record_deletion(
    state: &HandlerState,
    caller: &CallerIdentity,
    before: &JsonValue,
    after: &JsonValue,
) {
    let change = ConfigChange {
        param: LITELLM_SETTINGS,
        action: AuditAction::Deleted,
        table: AuditTable::Config,
        before: Some(before),
        after: Some(after),
        caller,
    };
    config_audit_log(state.config.audit.as_ref(), change).await;
}

fn callback_list(names: &[String]) -> JsonValue {
    let entries = names
        .iter()
        .map(|name| JsonValue::String(name.clone()))
        .collect();
    JsonValue::Array(entries)
}

fn callback_change(success: &[String]) -> JsonValue {
    let mut change = BTreeMap::new();
    change.insert(SUCCESS_CALLBACK.to_owned(), callback_list(success));
    JsonValue::Object(change.into_iter().collect())
}

// datetime.now().isoformat(): no offset suffix, and no fraction at all when the microseconds are zero
fn local_timestamp(moment: &DateTime<FixedOffset>) -> String {
    let local = moment.naive_local();
    if local.and_utc().timestamp_subsec_micros() == 0 {
        return local.format("%Y-%m-%dT%H:%M:%S").to_string();
    }
    local.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

// the caller's user_id, and the key's safe provenance hash under the name LiteLLM's audit row uses
fn audit_caller(
    identity: &Identity,
    principal: &Principal,
) -> Result<CallerIdentity, serde_json::Error> {
    let api_key = match principal.source_key.0.as_ref() {
        Some(key) => {
            let view: KeyProvenance = serde_json::from_value(serde_json::to_value(key)?)?;
            view.api_key
        }
        None => None,
    };
    Ok(CallerIdentity {
        user_id: identity.user_id.clone(),
        api_key,
        metadata: None,
        team_metadata: None,
    })
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KeyProvenance {
    api_key: Option<SecretString>,
}
