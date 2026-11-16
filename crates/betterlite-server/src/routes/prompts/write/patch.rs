use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use betterlite_prompts::registry::prompt_endpoint_contracts::records::{
    PatchPromptRequest, is_ambiguous_keyed_prompt_data,
};
use betterlite_prompts::registry::prompt_endpoint_contracts::rows::{PromptRow, PromptRowJson};
use betterlite_prompts::registry::prompt_endpoint_contracts::spec_conversion::create_versioned_prompt_spec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptColumn, PromptRowFilter, PromptRowOrder, PromptRowQuery, PromptRowUpdate,
    PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry_helpers::{
    get_base_prompt_id, get_version_number, prompt_environment_or_default,
};

use super::WriteResult;
use super::error::PromptWriteError;
use super::reject_config_prompt;
use super::state::{PromptWriteState, Writer};
use super::wire::{EnvironmentQuery, decode_body, masked};
use crate::auth::auth_identity::AuthContext;

pub(super) async fn find_patch_target<Table>(
    table: &Table,
    base_prompt_id: &str,
    requested_version: Option<i64>,
    environment: &str,
) -> WriteResult<PromptRow, Table>
where
    Table: PromptTableActions,
{
    let query = PromptRowQuery {
        filter: PromptRowFilter {
            id: None,
            prompt_id: Some(base_prompt_id.to_owned()),
            version: requested_version,
            environment: Some(environment.to_owned()),
        },
        order: vec![PromptRowOrder {
            column: PromptColumn::Version,
            descending: true,
        }],
        take: Some(1),
        distinct: Vec::new(),
    };
    let rows = table
        .find_many(query)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    rows.into_iter()
        .next()
        .ok_or_else(|| PromptWriteError::NotFound {
            detail: format!(
                "Prompt with ID {base_prompt_id} not found in environment {environment}"
            ),
        })
}

// A field the request leaves out keeps the stored value, and the primary key targets exactly one row
pub(super) async fn patch_row<Table>(
    writer: &Writer<'_, Table>,
    target_row: &PromptRow,
    request: PatchPromptRequest,
) -> WriteResult<PromptRow, Table>
where
    Table: PromptTableActions,
{
    let current_spec = create_versioned_prompt_spec(target_row)?;
    let litellm_params = request
        .litellm_params
        .unwrap_or(current_spec.litellm_params);
    let prompt_info = request.prompt_info.unwrap_or(current_spec.prompt_info);
    let update = PromptRowUpdate {
        litellm_params: PromptRowJson::Text(serde_json::to_string(&litellm_params)?),
        prompt_info: PromptRowJson::Text(serde_json::to_string(&prompt_info)?),
        created_by: writer.user_id.clone(),
    };
    let filter = PromptRowFilter {
        id: Some(target_row.id.clone()),
        prompt_id: None,
        version: None,
        environment: None,
    };
    let updated = writer
        .table
        .update(filter, update)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    updated.ok_or_else(|| PromptWriteError::NotFound {
        detail: format!(
            "Prompt with ID {} not found in environment {}",
            target_row.prompt_id, target_row.environment
        ),
    })
}

pub(super) async fn patch_prompt<Table>(
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
