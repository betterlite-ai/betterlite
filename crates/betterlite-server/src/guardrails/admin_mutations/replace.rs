use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use chrono::Utc;

use super::body::decode_write_request;
use super::error::{MANAGE_ADMIN_REQUIRED, MutationError};
use super::response::StoredGuardrail;
use super::review_rows::ReviewRows;
use super::sync::sync_in_memory;
use super::{MutationState, authenticated, require_admin};
use crate::auth::auth_identity::AuthContext;

pub(super) async fn replace_guardrail<R, S>(
    State(state): State<Arc<MutationState<R, S>>>,
    Path(guardrail_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<StoredGuardrail>, MutationError>
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    let context = authenticated(caller)?;
    let request =
        decode_write_request(&body).map_err(|error| MutationError::Body(Box::new(error)))?;
    require_admin(&context, MANAGE_ADMIN_REQUIRED)?;
    let database = state.database.as_ref().ok_or(MutationError::NoDatabase)?;
    let registry = database.runtime.registry();
    let existing = registry
        .get_guardrail_by_id_from_db(&guardrail_id)
        .await
        .map_err(|source| MutationError::Registry { source })?
        .ok_or_else(|| MutationError::NotFound {
            guardrail_id: guardrail_id.clone(),
        })?;
    let updated = registry
        .update_guardrail_in_db(&guardrail_id, &request.guardrail, Utc::now())
        .await
        .map_err(|source| MutationError::Registry { source })?;
    if let Err(error) = sync_in_memory(database, &guardrail_id, updated.clone()).await {
        registry
            .update_guardrail_in_db(&guardrail_id, &existing, Utc::now())
            .await
            .map_err(|source| MutationError::Registry { source })?;
        return Err(MutationError::UpdateRejected { source: error });
    }
    // Prisma's update returns every column; the review columns come from a second read of the row
    let review = database
        .review_rows
        .find_unique(&guardrail_id)
        .await
        .map_err(|source| MutationError::ReviewRows { source })?
        .ok_or_else(|| MutationError::MissingAfterUpdate {
            guardrail_id: guardrail_id.clone(),
        })?;
    Ok(Json(StoredGuardrail::from_rows(updated, review)))
}
