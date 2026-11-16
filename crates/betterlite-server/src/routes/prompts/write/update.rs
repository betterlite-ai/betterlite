use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::PromptTableActions;
use betterlite_prompts::registry::prompt_registry_helpers::get_base_prompt_id;

use super::state::PromptWriteState;
use super::wire::masked;
use super::{
    WriteResult, prompt_write_request, reject_config_prompt, require_any_version, store_new_version,
};
use crate::auth::auth_identity::AuthContext;

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
