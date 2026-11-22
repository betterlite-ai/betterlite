use std::sync::Arc;

use axum::extract::Extension;
use betterlite_core::api::api_boundaries::{ApiBoundariesInput, JsonBoundary};
use betterlite_core::api::claude_plugin_policy::{
    ClaudePluginPolicyError, PluginCaller, plugin_not_found_error, require_proxy_admin,
};
use betterlite_core::api::claude_plugin_write::ClaudePluginWriteInput;
use betterlite_db::auxiliary_repository::repository::TableError;

use super::ClaudePluginWriteState;
use super::error::ClaudePluginWriteError;
use super::store::ClaudePluginStore;
use crate::auth::auth_identity::AuthContext;

// LiteLLM authenticates the master key as PROXY_ADMIN; B003's stage carries it as this subject
const MASTER_KEY_SUBJECT: &str = "master_key";
// LiteLLM's litellm_proxy_admin_name, which the master key leaves in created_by
const MASTER_KEY_USER_ID: &str = "default_user_id";

pub(super) struct Gate {
    pub(super) caller: AuthContext,
    pub(super) store: Arc<dyn ClaudePluginStore>,
}

pub(super) fn gate(
    state: &ClaudePluginWriteState,
    extension: Option<Extension<AuthContext>>,
) -> Result<Gate, ClaudePluginWriteError> {
    let Some(Extension(caller)) = extension else {
        return Err(ClaudePluginWriteError::Unauthenticated);
    };
    require_admin(&caller)?;
    let store = state.connected_store()?;
    Ok(Gate { caller, store })
}

pub(super) fn prepare(
    state: &ClaudePluginWriteState,
    caller: Option<Extension<AuthContext>>,
    input: &ClaudePluginWriteInput,
) -> Result<Gate, ClaudePluginWriteError> {
    let checked = gate(state, caller)?;
    input.validate()?;
    Ok(checked)
}

// is_proxy_admin checks the PROXY_ADMIN role, which the master key's stage does not carry here
fn require_admin(context: &AuthContext) -> Result<(), ClaudePluginPolicyError> {
    if context.principal.subject == MASTER_KEY_SUBJECT {
        return Ok(());
    }
    require_proxy_admin(Some(&PluginCaller {
        principal: context.principal.clone(),
        object_permission: None,
        team_object_permission: None,
    }))
}

pub(super) fn creator_id(context: &AuthContext) -> Option<String> {
    if let Some(user) = context.principal.user.as_ref() {
        return Some(user.id.clone());
    }
    if context.principal.subject == MASTER_KEY_SUBJECT {
        return Some(MASTER_KEY_USER_ID.to_owned());
    }
    None
}

pub(super) const fn check_body(
    state: &ClaudePluginWriteState,
    body: &[u8],
) -> Result<(), ClaudePluginWriteError> {
    let boundary = ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: body.len(),
    });
    if state.limits.check(boundary).is_err() {
        return Err(ClaudePluginWriteError::TooLarge {
            observed: body.len(),
            limit: state.limits.max_json_bytes,
        });
    }
    Ok(())
}

pub(super) async fn require_present(
    store: &dyn ClaudePluginStore,
    plugin_name: &str,
    prefix: Option<&str>,
) -> Result<(), ClaudePluginWriteError> {
    let existing = store
        .find_by_name(plugin_name)
        .await
        .map_err(|source| table_failure(prefix, &source))?;
    if existing.is_none() {
        return Err(ClaudePluginWriteError::from(plugin_not_found_error(
            plugin_name,
        )));
    }
    Ok(())
}

// register, update and the lifecycle routes each wrap a table failure with their own prefix
pub(super) fn table_failure(prefix: Option<&str>, source: &TableError) -> ClaudePluginWriteError {
    match prefix {
        Some(label) => ClaudePluginWriteError::Backend {
            message: format!("{label}: {source}"),
        },
        None => ClaudePluginWriteError::Backend {
            message: source.to_string(),
        },
    }
}
