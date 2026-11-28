use std::sync::Arc;

use axum::extract::Extension;
use betterlite_core::api::api_boundaries::{ApiBoundariesInput, JsonBoundary};
use betterlite_core::api::claude_plugin_policy::{
    PluginCaller, plugin_not_found_error, require_proxy_admin,
};
use betterlite_core::api::claude_plugin_write::ClaudePluginWriteInput;
use betterlite_db::auxiliary_repository::repository::TableError;

use super::ClaudePluginWriteState;
use super::error::{BackendSource, ClaudePluginWriteError};
use super::store::ClaudePluginStore;
use crate::auth::auth_identity::AuthContext;

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
    require_proxy_admin(Some(&PluginCaller {
        principal: caller.principal.clone(),
        object_permission: None,
        team_object_permission: None,
    }))?;
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

// ast-grep-ignore: no-one-line-functions - B10171 cannot change flow.rs, the only caller
pub(super) fn creator_id(context: &AuthContext) -> Option<String> {
    context.principal.user.as_ref().map(|user| user.id.clone())
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
    prefix: Option<&'static str>,
) -> Result<(), ClaudePluginWriteError> {
    let existing = store
        .find_by_name(plugin_name)
        .await
        .map_err(|source| table_failure(prefix, source))?;
    if existing.is_none() {
        return Err(ClaudePluginWriteError::from(plugin_not_found_error(
            plugin_name,
        )));
    }
    Ok(())
}

// register, update and the lifecycle routes each wrap a table failure with their own prefix
pub(super) const fn table_failure(
    prefix: Option<&'static str>,
    source: TableError,
) -> ClaudePluginWriteError {
    ClaudePluginWriteError::Backend {
        prefix,
        source: BackendSource::Table(source),
    }
}
