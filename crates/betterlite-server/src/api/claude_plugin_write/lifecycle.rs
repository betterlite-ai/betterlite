use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use betterlite_core::api::claude_plugin_write::{
    ClaudePluginWriteAction, ClaudePluginWriteInput, ClaudePluginWriteResult, PluginMessage,
    SUCCESS,
};

use super::ClaudePluginWriteState;
use super::access::{gate, require_present, table_failure};
use super::error::ClaudePluginWriteError;
use super::wire::PluginPath;
use crate::auth::auth_identity::AuthContext;

// the two LiteLLM toggle routes share one body, so the route layer carries which way it goes
#[derive(Clone, Copy)]
pub(crate) enum PluginToggle {
    Enable,
    Disable,
}

pub(super) async fn toggle(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
    Extension(toggle): Extension<PluginToggle>,
) -> Result<Json<ClaudePluginWriteResult>, ClaudePluginWriteError> {
    let enabled = matches!(toggle, PluginToggle::Enable);
    let action = if enabled {
        ClaudePluginWriteAction::Enable
    } else {
        ClaudePluginWriteAction::Disable
    };
    let input = ClaudePluginWriteInput {
        action,
        plugin_name: path.plugin_name,
        spec: None,
    };
    let gate = gate(state.as_ref(), caller)?;
    input.validate()?;
    require_present(gate.store.as_ref(), &input.plugin_name, None).await?;
    let updated_at = state.clock.now().naive_utc();
    gate.store
        .set_enabled(&input.plugin_name, enabled, updated_at)
        .await
        .map_err(|source| table_failure(None, source))?;
    let state_word = if enabled { "enabled" } else { "disabled" };
    Ok(Json(ClaudePluginWriteResult::Message(PluginMessage {
        status: SUCCESS,
        message: format!("Plugin '{}' {state_word}", input.plugin_name),
    })))
}

pub(super) async fn remove(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
) -> Result<Json<ClaudePluginWriteResult>, ClaudePluginWriteError> {
    let input = ClaudePluginWriteInput {
        action: ClaudePluginWriteAction::Delete,
        plugin_name: path.plugin_name,
        spec: None,
    };
    let gate = gate(state.as_ref(), caller)?;
    input.validate()?;
    require_present(gate.store.as_ref(), &input.plugin_name, None).await?;
    gate.store
        .delete(&input.plugin_name)
        .await
        .map_err(|source| table_failure(None, source))?;
    Ok(Json(ClaudePluginWriteResult::Message(PluginMessage {
        status: SUCCESS,
        message: format!("Plugin '{}' deleted", input.plugin_name),
    })))
}
