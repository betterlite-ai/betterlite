use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, RawQuery, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::claude_plugin_policy::{
    PluginRecord, plugin_not_found_error, skill_visibility,
};
use betterlite_core::api::claude_plugin_read::error::plugin_not_granted_error;
use betterlite_core::api::claude_plugin_read::{
    ClaudePluginReadInput, ClaudePluginReadOperation, ClaudePluginReadResult, PluginManifestFields,
    PluginRow,
};

use super::access;
use super::error::ClaudePluginReadError;
use super::store::StoredPluginRow;
use super::wire::PluginListQuery;
use super::{ClaudePluginReadState, decode_claude_plugin_read};
use crate::auth::auth_identity::AuthContext;
use crate::model_query::read_query;

// the routes take different path parameters, and Option<Path<T>> rejects the parameterless one
pub(super) async fn read_list(
    State(state): State<Arc<ClaudePluginReadState>>,
    caller: Option<Extension<AuthContext>>,
    RawQuery(raw_query): RawQuery,
) -> Response {
    let (_, [enabled_only]) = match read_query(raw_query.as_deref(), ["enabled_only"]) {
        Ok(flags) => flags,
        Err(error) => return error.into_response(),
    };
    let query = PluginListQuery { enabled_only };
    match list_plugins(state.as_ref(), caller, query).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => error.into_response(),
    }
}

pub(super) async fn read_plugin(
    State(state): State<Arc<ClaudePluginReadState>>,
    caller: Option<Extension<AuthContext>>,
    Path(plugin_name): Path<String>,
) -> Response {
    match get_plugin(state.as_ref(), caller, &plugin_name).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn list_plugins(
    state: &ClaudePluginReadState,
    caller: Option<Extension<AuthContext>>,
    query: PluginListQuery,
) -> Result<ClaudePluginReadResult, ClaudePluginReadError> {
    let store = state.connected_store()?;
    let input = ClaudePluginReadInput {
        operation: ClaudePluginReadOperation::List {
            enabled_only: query.enabled_only,
        },
        caller: access::caller(caller)?,
    };
    let filter = input.list_filter();
    let stored = store.list(&filter).await?;
    // list_plugins parses each manifest only after visibility.allows passes, so the decode waits too
    let visibility = skill_visibility(input.caller.as_ref());
    let mut rows = Vec::with_capacity(stored.len());
    for row in stored {
        if !visibility.allows(&record(&row)) {
            continue;
        }
        rows.push(plugin_row(row, &state.limits)?);
    }
    input.run(rows).map_err(ClaudePluginReadError::from)
}

async fn get_plugin(
    state: &ClaudePluginReadState,
    caller: Option<Extension<AuthContext>>,
    plugin_name: &str,
) -> Result<ClaudePluginReadResult, ClaudePluginReadError> {
    let store = state.connected_store()?;
    let input = ClaudePluginReadInput {
        operation: ClaudePluginReadOperation::Get {
            plugin_name: plugin_name.to_owned(),
        },
        caller: access::caller(caller)?,
    };
    let found = store.find_by_name(plugin_name).await?;
    let Some(row) = found else {
        return Err(ClaudePluginReadError::from(plugin_not_found_error(
            plugin_name,
        )));
    };
    // get_plugin checks the grant before it parses the manifest, so an unreadable one still answers 403
    let visibility = skill_visibility(input.caller.as_ref());
    if !visibility.allows(&record(&row)) {
        return Err(ClaudePluginReadError::from(plugin_not_granted_error(
            plugin_name,
        )));
    }
    let decoded = plugin_row(row, &state.limits)?;
    input
        .run(vec![decoded])
        .map_err(ClaudePluginReadError::from)
}

fn record(row: &StoredPluginRow) -> PluginRecord {
    PluginRecord {
        name: row.name.clone(),
        enabled: row.enabled,
    }
}

pub(super) fn plugin_row(
    row: StoredPluginRow,
    limits: &ApiLimits,
) -> Result<PluginRow, ClaudePluginReadError> {
    let manifest_text = row.manifest_json.as_deref().unwrap_or_default();
    let manifest = if manifest_text.is_empty() {
        PluginManifestFields::default()
    } else {
        decode_claude_plugin_read(manifest_text.as_bytes(), limits)?
    };
    Ok(PluginRow {
        id: row.id,
        name: row.name,
        version: row.version,
        description: row.description,
        enabled: row.enabled,
        created_at: row.created_at,
        updated_at: row.updated_at,
        created_by: row.created_by,
        manifest,
    })
}
