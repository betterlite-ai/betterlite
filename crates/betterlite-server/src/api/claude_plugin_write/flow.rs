use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_core::api::claude_plugin_policy::source::{
    PluginSource, PluginSpec, UpdatePluginSpec, build_plugin_manifest,
};
use betterlite_core::api::claude_plugin_policy::{name_conflict_error, plugin_not_found_error};
use betterlite_core::api::claude_plugin_write::{
    ClaudePluginWriteAction, ClaudePluginWriteFailure, ClaudePluginWriteInput,
    ClaudePluginWriteResult, PluginRegistration, PluginView, REGISTRATION_FAILED,
    RegistrationAction, SUCCESS, StoredPlugin, UPDATE_FAILED,
};
use serde::de::DeserializeOwned;

use super::ClaudePluginWriteState;
use super::access::{Gate, check_body, creator_id, prepare, require_present, table_failure};
use super::error::ClaudePluginWriteError;
use super::store::{ClaudePluginStore, NewPlugin, PluginChanges, PluginCreateError};
use super::wire::{CreatePluginBody, PluginPath};
use crate::auth::auth_identity::AuthContext;
use crate::search_tool_routes::body::decode_body;

fn create_failure(error: &PluginCreateError, plugin_name: &str) -> ClaudePluginWriteError {
    match error {
        PluginCreateError::NameTaken => {
            ClaudePluginWriteError::from(name_conflict_error(plugin_name))
        }
        PluginCreateError::Table(source) => table_failure(Some(REGISTRATION_FAILED), source),
    }
}

fn decode_write<T>(state: &ClaudePluginWriteState, body: &[u8]) -> Result<T, ClaudePluginWriteError>
where
    T: DeserializeOwned,
{
    check_body(state, body)?;
    decode_body(body).map_err(ClaudePluginWriteError::from)
}

fn create_input(
    state: &ClaudePluginWriteState,
    body: &[u8],
) -> Result<ClaudePluginWriteInput, ClaudePluginWriteError> {
    let request: CreatePluginBody = decode_write(state, body)?;
    Ok(ClaudePluginWriteInput {
        action: ClaudePluginWriteAction::Create,
        plugin_name: request.name,
        spec: Some(request.spec),
    })
}

fn update_write(plugin_name: String, spec: PluginSpec) -> ClaudePluginWriteInput {
    ClaudePluginWriteInput {
        action: ClaudePluginWriteAction::Update,
        plugin_name,
        spec: Some(spec),
    }
}

async fn ensure_absent(
    store: &dyn ClaudePluginStore,
    plugin_name: &str,
) -> Result<(), ClaudePluginWriteError> {
    let existing = store
        .find_by_name(plugin_name)
        .await
        .map_err(|source| table_failure(Some(REGISTRATION_FAILED), &source))?;
    if existing.is_none() {
        return Ok(());
    }
    Err(ClaudePluginWriteError::from(name_conflict_error(
        plugin_name,
    )))
}

async fn register_plugin(
    state: &ClaudePluginWriteState,
    gate: &Gate,
    input: &ClaudePluginWriteInput,
) -> Result<(StoredPlugin, PluginSource), ClaudePluginWriteError> {
    ensure_absent(gate.store.as_ref(), &input.plugin_name).await?;
    let (write, source) = new_plugin(state, input, creator_id(&gate.caller))?;
    let created = gate
        .store
        .create(write)
        .await
        .map_err(|error| create_failure(&error, &input.plugin_name))?;
    Ok((created, source))
}

async fn require_updated(
    store: &dyn ClaudePluginStore,
    changes: PluginChanges,
    plugin_name: &str,
) -> Result<StoredPlugin, ClaudePluginWriteError> {
    let found = store
        .update(changes)
        .await
        .map_err(|source| table_failure(Some(UPDATE_FAILED), &source))?;
    found.ok_or_else(|| ClaudePluginWriteError::from(plugin_not_found_error(plugin_name)))
}

fn registration(
    stored: StoredPlugin,
    source: PluginSource,
    action: RegistrationAction,
) -> ClaudePluginWriteResult {
    ClaudePluginWriteResult::Registration(Box::new(PluginRegistration {
        status: SUCCESS,
        action,
        plugin: PluginView {
            id: stored.id,
            name: stored.name,
            version: stored.version,
            description: stored.description,
            source,
            enabled: stored.enabled,
        },
    }))
}

pub(super) async fn create(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<ClaudePluginWriteResult>, ClaudePluginWriteError> {
    let input = create_input(state.as_ref(), &body)?;
    let gate = prepare(state.as_ref(), caller, &input)?;
    let (created, source) = register_plugin(state.as_ref(), &gate, &input).await?;
    Ok(Json(registration(
        created,
        source,
        RegistrationAction::Created,
    )))
}

fn new_plugin(
    state: &ClaudePluginWriteState,
    input: &ClaudePluginWriteInput,
    created_by: Option<String>,
) -> Result<(NewPlugin, PluginSource), ClaudePluginWriteError> {
    let fields = input.fields()?;
    let manifest_json = manifest_json(input, REGISTRATION_FAILED)?;
    let now = state.clock.now().naive_utc();
    let write = NewPlugin {
        name: input.plugin_name.clone(),
        version: fields.version,
        description: fields.description,
        manifest_json,
        enabled: true,
        created_at: now,
        updated_at: now,
        created_by,
    };
    Ok((write, fields.source))
}

pub(super) async fn update(
    State(state): State<Arc<ClaudePluginWriteState>>,
    caller: Option<Extension<AuthContext>>,
    Path(path): Path<PluginPath>,
    body: Bytes,
) -> Result<Json<ClaudePluginWriteResult>, ClaudePluginWriteError> {
    let request: UpdatePluginSpec = decode_write(state.as_ref(), &body)?;
    let input = update_write(path.plugin_name, PluginSpec::from(request));
    let gate = prepare(state.as_ref(), caller, &input)?;
    require_present(gate.store.as_ref(), &input.plugin_name, Some(UPDATE_FAILED)).await?;
    let (changes, source) = plugin_changes(state.as_ref(), &input)?;
    let stored = require_updated(gate.store.as_ref(), changes, &input.plugin_name).await?;
    Ok(Json(registration(
        stored,
        source,
        RegistrationAction::Updated,
    )))
}

fn plugin_changes(
    state: &ClaudePluginWriteState,
    input: &ClaudePluginWriteInput,
) -> Result<(PluginChanges, PluginSource), ClaudePluginWriteError> {
    let fields = input.fields()?;
    let manifest_json = manifest_json(input, UPDATE_FAILED)?;
    let changes = PluginChanges {
        name: input.plugin_name.clone(),
        version: fields.version,
        description: fields.description,
        manifest_json,
        updated_at: state.clock.now().naive_utc(),
    };
    Ok((changes, fields.source))
}

fn manifest_json(
    input: &ClaudePluginWriteInput,
    prefix: &str,
) -> Result<String, ClaudePluginWriteError> {
    let spec = input
        .spec
        .as_ref()
        .ok_or(ClaudePluginWriteFailure::MissingSpec)?;
    let manifest = build_plugin_manifest(&input.plugin_name, spec);
    serde_json::to_string(&manifest).map_err(|source| ClaudePluginWriteError::Backend {
        message: format!("{prefix}: {source}"),
    })
}
