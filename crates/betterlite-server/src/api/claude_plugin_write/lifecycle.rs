use axum::extract::Extension;
use betterlite_core::api::claude_plugin_write::{
    ClaudePluginWriteAction, ClaudePluginWriteInput, ClaudePluginWriteResult, PluginMessage,
    SUCCESS,
};

use super::ClaudePluginWriteState;
use super::access::{gate, require_present, table_failure};
use super::error::ClaudePluginWriteError;
use crate::auth::auth_identity::AuthContext;

pub(super) async fn set_enabled(
    state: &ClaudePluginWriteState,
    caller: Option<Extension<AuthContext>>,
    plugin_name: String,
    enabled: bool,
) -> Result<ClaudePluginWriteResult, ClaudePluginWriteError> {
    let action = if enabled {
        ClaudePluginWriteAction::Enable
    } else {
        ClaudePluginWriteAction::Disable
    };
    let input = ClaudePluginWriteInput {
        action,
        plugin_name,
        spec: None,
    };
    let gate = gate(state, caller)?;
    input.validate()?;
    require_present(gate.store.as_ref(), &input.plugin_name, None).await?;
    let updated_at = state.clock.now().naive_utc();
    gate.store
        .set_enabled(&input.plugin_name, enabled, updated_at)
        .await
        .map_err(|source| table_failure(None, &source))?;
    let state_word = if enabled { "enabled" } else { "disabled" };
    Ok(ClaudePluginWriteResult::Message(PluginMessage {
        status: SUCCESS,
        message: format!("Plugin '{}' {state_word}", input.plugin_name),
    }))
}

pub(super) async fn remove(
    state: &ClaudePluginWriteState,
    caller: Option<Extension<AuthContext>>,
    plugin_name: String,
) -> Result<ClaudePluginWriteResult, ClaudePluginWriteError> {
    let input = ClaudePluginWriteInput {
        action: ClaudePluginWriteAction::Delete,
        plugin_name,
        spec: None,
    };
    let gate = gate(state, caller)?;
    input.validate()?;
    require_present(gate.store.as_ref(), &input.plugin_name, None).await?;
    gate.store
        .delete(&input.plugin_name)
        .await
        .map_err(|source| table_failure(None, &source))?;
    Ok(ClaudePluginWriteResult::Message(PluginMessage {
        status: SUCCESS,
        message: format!("Plugin '{}' deleted", input.plugin_name),
    }))
}
