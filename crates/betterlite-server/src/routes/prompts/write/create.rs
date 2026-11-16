use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use betterlite_models::prompt_spec::PromptSpec;
use betterlite_prompts::registry::prompt_endpoint_contracts::table::PromptTableActions;

use super::state::PromptWriteState;
use super::wire::masked;
use super::{WriteResult, prompt_write_request, store_new_version};
use crate::auth::auth_identity::AuthContext;

pub(super) async fn create_prompt<Table>(
    State(state): State<Arc<PromptWriteState<Table>>>,
    Extension(context): Extension<AuthContext>,
    body: Bytes,
) -> WriteResult<Json<PromptSpec>, Table>
where
    Table: PromptTableActions + 'static,
{
    let (request, writer) = prompt_write_request(&state, &context, &body, "create")?;
    let spec = store_new_version(&state, &writer, &request.prompt_id, &request).await?;
    Ok(masked(spec))
}
