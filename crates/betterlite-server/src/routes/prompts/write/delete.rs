use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use betterlite_prompts::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptRowFilter, PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry_helpers::get_base_prompt_id;

use super::error::PromptWriteError;
use super::state::{PromptWriteState, Writer};
use super::wire::{DeletedPrompt, EnvironmentQuery};
use super::{WriteResult, reject_config_prompt};
use crate::auth::auth_identity::AuthContext;

async fn require_resolvable<Table>(
    state: &PromptWriteState<Table>,
    prompt_id: &str,
    environment: Option<&str>,
) -> WriteResult<(), Table>
where
    Table: PromptTableActions,
{
    let resolved = {
        let registry = state.registry.lock().await;
        registry
            .resolve_prompt_spec(prompt_id, None, environment)
            .is_some()
    };
    if !resolved {
        return Err(PromptWriteError::NotFound {
            detail: format!("Prompt with ID {prompt_id} not found"),
        });
    }
    Ok(())
}

async fn delete_rows<Table>(
    writer: &Writer<'_, Table>,
    base_prompt_id: &str,
    environment: Option<String>,
) -> WriteResult<(), Table>
where
    Table: PromptTableActions,
{
    let filter = PromptRowFilter {
        id: None,
        prompt_id: Some(base_prompt_id.to_owned()),
        version: None,
        environment,
    };
    writer
        .table
        .delete_many(filter)
        .await
        .map_err(|source| PromptWriteError::Table { source })?;
    Ok(())
}

async fn sync_registry<Table>(
    state: &PromptWriteState<Table>,
    base_prompt_id: &str,
    environment: Option<&str>,
) where
    Table: PromptTableActions,
{
    let mut registry = state.registry.lock().await;
    let mut callbacks = state.callbacks.lock().await;
    registry.delete_prompts_by_base_id(base_prompt_id, environment, &mut *callbacks);
}

pub(super) async fn delete_prompt<Table>(
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
    require_resolvable(&state, &prompt_id, environment.as_deref()).await?;
    reject_config_prompt(&state, &base_prompt_id, "delete").await?;
    delete_rows(&writer, &base_prompt_id, environment.clone()).await?;
    sync_registry(&state, &base_prompt_id, environment.as_deref()).await;
    let environment_note = environment.map_or_else(String::new, |name| format!(" from {name}"));
    Ok(Json(DeletedPrompt {
        message: format!("Prompt {base_prompt_id} deleted successfully{environment_note}"),
    }))
}
