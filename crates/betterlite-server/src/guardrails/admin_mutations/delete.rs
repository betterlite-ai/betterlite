use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use betterlite_guardrails::guardrail_registry::registry::DeleteMessage;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;

use super::error::{MANAGE_ADMIN_REQUIRED, MutationError};
use super::review_rows::ReviewRows;
use super::{MutationState, authenticated, mutation_database, require_admin};
use crate::auth::auth_identity::AuthContext;

// LiteLLM logs 'Unknown' because the delete result holds only the message
const DELETED_NAME: &str = "Unknown";

pub(super) async fn delete_guardrail<R, S>(
    State(state): State<Arc<MutationState<R, S>>>,
    Path(guardrail_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<DeleteMessage>, MutationError>
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    let context = authenticated(caller)?;
    require_admin(&context, MANAGE_ADMIN_REQUIRED)?;
    let database = mutation_database(&state)?;
    let registry = database.runtime.registry();
    let existing = registry
        .get_guardrail_by_id_from_db(&guardrail_id)
        .await
        .map_err(MutationError::internal)?;
    if existing.is_none() {
        return Err(MutationError::NotFound { guardrail_id });
    }
    let message = registry
        .delete_guardrail_from_db(&guardrail_id)
        .await
        .map_err(MutationError::internal)?;
    database
        .runtime
        .handler()
        .write()
        .await
        .delete_in_memory_guardrail(&guardrail_id);
    tracing::info!(
        "Immediate sync: Successfully removed guardrail '{DELETED_NAME}' (ID: {guardrail_id}) from memory"
    );
    Ok(Json(message))
}
