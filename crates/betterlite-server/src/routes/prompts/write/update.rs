use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::{
    PromptRowFilter, PromptRowQuery, PromptTableActions,
};
use betterlite_prompts::registry::prompt_registry_helpers::get_base_prompt_id;

use super::error::PromptWriteError;
use super::state::PromptWriteState;
use super::wire::masked;
use super::{WriteResult, prompt_write_request, reject_config_prompt, store_new_version};
use crate::auth::auth_identity::AuthContext;

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

pub(super) async fn update_prompt<Table>(
    State(state): State<Arc<PromptWriteState<Table>>>,
    Path(prompt_id): Path<String>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> WriteResult<Json<PromptSpec>, Table>
where
    Table: PromptTableActions + 'static,
{
    let (request, writer) = prompt_write_request(&state, &context, &body, "update")?;
    let base_prompt_id = get_base_prompt_id(&prompt_id);
    require_any_version(writer.table, &base_prompt_id).await?;
    reject_config_prompt(&state, &base_prompt_id, "update").await?;
    let spec = store_new_version(&state, &writer, &base_prompt_id, &request).await?;
    Ok(masked(spec))
}
