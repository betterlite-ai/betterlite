use axum::extract::Extension;
use betterlite_core::api::claude_plugin_policy::source::{
    PluginSource, PluginSpec, UpdatePluginSpec,
};
use betterlite_core::api::claude_plugin_policy::{name_conflict_error, plugin_not_found_error};
use betterlite_core::api::claude_plugin_write::{
    ClaudePluginWriteAction, ClaudePluginWriteInput, ClaudePluginWriteResult, PluginRegistration,
    PluginView, REGISTRATION_FAILED, RegistrationAction, SUCCESS, UPDATE_FAILED,
};

use super::ClaudePluginWriteState;
use super::access::{check_body, creator_id, gate, require_present, table_failure};
use super::error::ClaudePluginWriteError;
use super::store::{ClaudePluginStore, NewPlugin, PluginChanges, PluginCreateError};
use super::wire::CreatePluginBody;
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

fn create_input(
    state: &ClaudePluginWriteState,
    body: &[u8],
) -> Result<ClaudePluginWriteInput, ClaudePluginWriteError> {
    check_body(state, body)?;
    let request: CreatePluginBody = decode_body(body)?;
    Ok(ClaudePluginWriteInput {
        action: ClaudePluginWriteAction::Create,
        plugin_name: request.name,
        spec: Some(request.spec),
    })
}

fn update_input(
    state: &ClaudePluginWriteState,
    plugin_name: String,
    body: &[u8],
) -> Result<ClaudePluginWriteInput, ClaudePluginWriteError> {
    check_body(state, body)?;
    let request: UpdatePluginSpec = decode_body(body)?;
    Ok(ClaudePluginWriteInput {
        action: ClaudePluginWriteAction::Update,
        plugin_name,
        spec: Some(PluginSpec::from(request)),
    })
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

pub(super) async fn create(
    state: &ClaudePluginWriteState,
    caller: Option<Extension<AuthContext>>,
    body: &[u8],
) -> Result<ClaudePluginWriteResult, ClaudePluginWriteError> {
    let input = create_input(state, body)?;
    let gate = gate(state, caller)?;
    input.validate()?;
    ensure_absent(gate.store.as_ref(), &input.plugin_name).await?;
    let (write, source) = new_plugin(state, &input, creator_id(&gate.caller))?;
    let created = gate
        .store
        .create(write)
        .await
        .map_err(|error| create_failure(&error, &input.plugin_name))?;
    Ok(ClaudePluginWriteResult::Registration(PluginRegistration {
        status: SUCCESS,
        action: RegistrationAction::Created,
        plugin: PluginView {
            id: created.id,
            name: created.name,
            version: created.version,
            description: created.description,
            source,
            enabled: created.enabled,
        },
    }))
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
    state: &ClaudePluginWriteState,
    caller: Option<Extension<AuthContext>>,
    plugin_name: String,
    body: &[u8],
) -> Result<ClaudePluginWriteResult, ClaudePluginWriteError> {
    let input = update_input(state, plugin_name, body)?;
    let gate = gate(state, caller)?;
    input.validate()?;
    require_present(gate.store.as_ref(), &input.plugin_name, Some(UPDATE_FAILED)).await?;
    let (changes, source) = plugin_changes(state, &input)?;
    let changed = gate
        .store
        .update(changes)
        .await
        .map_err(|source| table_failure(Some(UPDATE_FAILED), &source))?;
    let Some(stored) = changed else {
        return Err(ClaudePluginWriteError::from(plugin_not_found_error(
            &input.plugin_name,
        )));
    };
    Ok(ClaudePluginWriteResult::Registration(PluginRegistration {
        status: SUCCESS,
        action: RegistrationAction::Updated,
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
    let manifest = input.manifest()?;
    serde_json::to_string(&manifest).map_err(|source| ClaudePluginWriteError::Backend {
        message: format!("{prefix}: {source}"),
    })
}
