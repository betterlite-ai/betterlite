#![cfg_attr(
    not(test),
    expect(dead_code, reason = "B8672 mounts the prompt write routes")
)]

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::routing::{post, put};
use axum::{Json, Router};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use betterlite_prompts::registry::prompt_endpoint_contracts::records::{
    PatchPromptRequest, Prompt, is_ambiguous_keyed_prompt_data,
};
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::create_versioned_prompt_spec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptRowFilter, PromptRowQuery, PromptTableActions, get_next_version_for_prompt,
};
use betterlite_prompts::registry::prompt_registry_helpers::{
    get_base_prompt_id, get_version_number, prompt_environment_or_default,
};

use self::create::create_prompt;
use self::error::PromptWriteError;
use self::patch::{find_patch_target, patch_row};
use self::state::{PromptWriteState, Writer};
use self::update::update_prompt;
use self::wire::{DeletedPrompt, EnvironmentQuery, decode_body, masked, new_row};
use crate::auth::auth_identity::AuthContext;

mod create;
mod error;
mod patch;
pub(crate) mod state;
mod tests;
mod update;
mod wire;

type WriteResult<Value, Table> =
    Result<Value, PromptWriteError<<Table as PromptTableActions>::Failure>>;

// every handler first answers 403 for a non-admin and 500 for a missing database, as the guard does
pub(crate) fn routes<Table>(state: Arc<PromptWriteState<Table>>) -> Router
where
    Table: PromptTableActions + 'static,
{
    Router::new()
        .route("/prompts", post(create_prompt::<Table>))
        .route(
            "/prompts/{prompt_id}",
            put(update_prompt::<Table>)
                .patch(patch_prompt::<Table>)
                .delete(delete_prompt::<Table>),
        )
        .with_state(state)
}

// POST and PUT decode the body, ask the guard for 403 and 500, then reject ambiguous data
fn prompt_write_request<'state, Table>(
    state: &'state PromptWriteState<Table>,
    context: &AuthContext,
    body: &[u8],
    action: &'static str,
) -> WriteResult<(Prompt, Writer<'state, Table>), Table>
where
    Table: PromptTableActions,
{
    let request: Prompt = decode_body(body)?;
    let writer = state.require_admin_and_db(context, action)?;
    if is_ambiguous_keyed_prompt_data(&request.litellm_params) {
        return Err(PromptWriteError::Ambiguous);
    }
    Ok((request, writer))
}

async fn require_any_version<Table>(table: &Table, base_prompt_id: &str) -> WriteResult<(), Table>
where
    Table: PromptTableActions,
{
    let query = PromptRowQuery {
        filter: PromptRowFilter {
            id: None,
            prompt_id: Some(base_prompt_id.to_owned()),
            version: None,
            environment: None,
        },
        order: Vec::new(),
        take: None,
        distinct: Vec::new(),
    };
    let existing = table
        .find_many(query)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    if existing.is_empty() {
        return Err(PromptWriteError::NotFound {
            detail: format!("Prompt with ID {base_prompt_id} not found"),
        });
    }
    Ok(())
}

async fn store_new_version<Table>(
    state: &PromptWriteState<Table>,
    writer: &Writer<'_, Table>,
    prompt_id: &str,
    request: &Prompt,
) -> WriteResult<PromptSpec, Table>
where
    Table: PromptTableActions,
{
    let environment = prompt_environment_or_default(
        request
            .prompt_info
            .as_ref()
            .map(|info| info.environment.as_str()),
    );
    let version = get_next_version_for_prompt(writer.table, prompt_id, Some(&environment))
        .await
        .map_err(|source| PromptWriteError::Version { source })?;
    let row_data = new_row(
        prompt_id,
        version,
        environment,
        writer.user_id.clone(),
        request,
    )?;
    let row = writer
        .table
        .create(row_data)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    let spec = create_versioned_prompt_spec(&row)?;
    // the database row stays when the registry cannot initialize it, and the caller sees a 500
    let mut registry = state.registry.lock().await;
    let mut callbacks = state.callbacks.lock().await;
    registry
        .initialize_prompt(&spec, None, &mut *callbacks)
        .map_err(PromptWriteError::from)
}

async fn reject_config_prompt<Table>(
    state: &PromptWriteState<Table>,
    base_prompt_id: &str,
    action: &'static str,
) -> WriteResult<(), Table>
where
    Table: PromptTableActions,
{
    let is_config = {
        let registry = state.registry.lock().await;
        registry.has_config_prompt(base_prompt_id)
    };
    if is_config {
        return Err(PromptWriteError::ConfigPrompt { action });
    }
    Ok(())
}

async fn patch_prompt<Table>(
    State(state): State<Arc<PromptWriteState<Table>>>,
    Path(prompt_id): Path<String>,
    Query(query): Query<EnvironmentQuery>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> WriteResult<Json<PromptSpec>, Table>
where
    Table: PromptTableActions + 'static,
{
    let request: PatchPromptRequest = decode_body(&body)?;
    let writer = state.require_admin_and_db(&context, "patch")?;
    if request
        .litellm_params
        .as_ref()
        .is_some_and(is_ambiguous_keyed_prompt_data)
    {
        return Err(PromptWriteError::Ambiguous);
    }
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    let requested_version = (prompt_id != base_prompt_id).then(|| get_version_number(&prompt_id));
    let environment = prompt_environment_or_default(query.environment.as_deref());
    let target_row = find_patch_target(
        writer.table,
        &base_prompt_id,
        requested_version,
        &environment,
    )
    .await?;
    reject_config_prompt(&state, &base_prompt_id, "update").await?;
    let updated_row = patch_row(&writer, &target_row, request).await?;
    let updated_spec = create_versioned_prompt_spec(&updated_row)?;
    let reloaded = {
        let mut registry = state.registry.lock().await;
        let mut callbacks = state.callbacks.lock().await;
        registry.reload_prompt(&updated_spec, &mut *callbacks)?
    };
    Ok(masked(reloaded))
}

async fn delete_prompt<Table>(
    State(state): State<Arc<PromptWriteState<Table>>>,
    Path(prompt_id): Path<String>,
    Query(query): Query<EnvironmentQuery>,
    Extension(context): Extension<AuthContext>,
) -> WriteResult<Json<DeletedPrompt>, Table>
where
    Table: PromptTableActions + 'static,
{
    let writer = state.require_admin_and_db(&context, "delete")?;
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    let environment = query.environment;
    let resolved = {
        let registry = state.registry.lock().await;
        registry
            .resolve_prompt_spec(&prompt_id, None, environment.as_deref())
            .is_some()
    };
    if !resolved {
        return Err(PromptWriteError::NotFound {
            detail: format!("Prompt with ID {prompt_id} not found"),
        });
    }
    reject_config_prompt(&state, &base_prompt_id, "delete").await?;
    let filter = PromptRowFilter {
        id: None,
        prompt_id: Some(base_prompt_id.clone()),
        version: None,
        environment: environment.clone(),
    };
    writer
        .table
        .delete_many(filter)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    let mut registry = state.registry.lock().await;
    let mut callbacks = state.callbacks.lock().await;
    registry.delete_prompts_by_base_id(&base_prompt_id, environment.as_deref(), &mut *callbacks);
    drop(callbacks);
    drop(registry);
    let environment_note = environment.map_or_else(String::new, |name| format!(" from {name}"));
    Ok(Json(DeletedPrompt {
        message: format!("Prompt {base_prompt_id} deleted successfully{environment_note}"),
    }))
}
