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

use self::error::PromptWriteError;
use self::patch::{find_patch_target, patch_row};
use self::state::{PromptWriteState, Writer};
use self::wire::{DeletedPrompt, EnvironmentQuery, decode_body, masked, new_row};
use crate::auth::auth_identity::AuthContext;

mod error;
mod patch;
pub(crate) mod state;
mod tests;
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

async fn create_prompt<Table>(
    State(state): State<Arc<PromptWriteState<Table>>>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> WriteResult<Json<PromptSpec>, Table>
where
    Table: PromptTableActions + 'static,
{
    let request: Prompt = decode_body(&body)?;
    let writer = state.require_admin_and_db(&context, "create")?;
    if is_ambiguous_keyed_prompt_data(&request.litellm_params) {
        return Err(PromptWriteError::Ambiguous);
    }
    let spec = store_new_version(&state, &writer, &request.prompt_id, &request).await?;
    Ok(masked(spec))
}

async fn update_prompt<Table>(
    State(state): State<Arc<PromptWriteState<Table>>>,
    Path(prompt_id): Path<String>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> WriteResult<Json<PromptSpec>, Table>
where
    Table: PromptTableActions + 'static,
{
    let request: Prompt = decode_body(&body)?;
    let writer = state.require_admin_and_db(&context, "update")?;
    if is_ambiguous_keyed_prompt_data(&request.litellm_params) {
        return Err(PromptWriteError::Ambiguous);
    }
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    let query = PromptRowQuery {
        filter: PromptRowFilter {
            id: None,
            prompt_id: Some(base_prompt_id.clone()),
            version: None,
            environment: None,
        },
        order: Vec::new(),
        take: None,
        distinct: Vec::new(),
    };
    let existing = writer
        .table
        .find_many(query)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    if existing.is_empty() {
        return Err(PromptWriteError::NotFound {
            detail: format!("Prompt with ID {base_prompt_id} not found"),
        });
    }
    if state
        .registry
        .lock()
        .await
        .has_config_prompt(&base_prompt_id)
    {
        return Err(PromptWriteError::ConfigPrompt { action: "update" });
    }
    let spec = store_new_version(&state, &writer, &base_prompt_id, &request).await?;
    Ok(masked(spec))
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
    if state
        .registry
        .lock()
        .await
        .has_config_prompt(&base_prompt_id)
    {
        return Err(PromptWriteError::ConfigPrompt { action: "update" });
    }
    let updated_row = patch_row(&writer, &target_row, request).await?;
    let updated_spec = create_versioned_prompt_spec(&updated_row)?;
    let reloaded = {
        let mut registry = state.registry.lock().await;
        let mut callbacks = state.callbacks.lock().await;
        registry.reload_prompt(&updated_spec, &mut *callbacks)?
    };
    Ok(masked(reloaded))
}

async fn require_deletable<Table>(
    state: &PromptWriteState<Table>,
    prompt_id: &str,
    environment: Option<&str>,
) -> WriteResult<(), Table>
where
    Table: PromptTableActions,
{
    let registry = state.registry.lock().await;
    if registry
        .resolve_prompt_spec(prompt_id, None, environment)
        .is_none()
    {
        return Err(PromptWriteError::NotFound {
            detail: format!("Prompt with ID {prompt_id} not found"),
        });
    }
    if registry.has_config_prompt(&get_base_prompt_id(prompt_id)) {
        return Err(PromptWriteError::ConfigPrompt { action: "delete" });
    }
    Ok(())
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
    require_deletable(&state, &prompt_id, environment.as_deref()).await?;
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
    {
        let mut registry = state.registry.lock().await;
        let mut callbacks = state.callbacks.lock().await;
        registry.delete_prompts_by_base_id(
            &base_prompt_id,
            environment.as_deref(),
            &mut *callbacks,
        );
    }
    let environment_note = environment.map_or_else(String::new, |name| format!(" from {name}"));
    Ok(Json(DeletedPrompt {
        message: format!("Prompt {base_prompt_id} deleted successfully{environment_note}"),
    }))
}
