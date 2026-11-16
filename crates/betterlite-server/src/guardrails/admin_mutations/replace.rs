use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_guardrails::guardrail_startup::reconcile::GuardrailReconcile as _;
use chrono::Utc;

use super::body::decode_write_request;
use super::error::{MANAGE_ADMIN_REQUIRED, MutationError, is_configuration_error};
use super::response::StoredGuardrail;
use super::review_rows::ReviewRows;
use super::{MutationState, authenticated, mutation_database, require_admin};
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
    let request = decode_write_request(&body).map_err(MutationError::body)?;
    require_admin(&context, MANAGE_ADMIN_REQUIRED)?;
    let database = mutation_database(&state)?;
    let registry = database.runtime.registry();
    let existing = registry
        .get_guardrail_by_id_from_db(&guardrail_id)
        .await
        .map_err(MutationError::internal)?;
    let Some(existing) = existing else {
        return Err(MutationError::NotFound { guardrail_id });
    };
    let updated = registry
        .update_guardrail_in_db(&guardrail_id, &request.guardrail, Utc::now())
        .await
        .map_err(MutationError::internal)?;
    let name = updated.guardrail_name.clone();
    let synced = database
        .runtime
        .handler()
        .write()
        .await
        .sync_guardrail_from_db(updated.clone());
    match synced {
        Ok(_held) => tracing::info!(
            "Immediate sync: Successfully updated guardrail '{name}' (ID: {guardrail_id})"
        ),
        Err(error) if is_configuration_error(&error) => {
            registry
                .update_guardrail_in_db(&guardrail_id, &existing, Utc::now())
                .await
                .map_err(MutationError::internal)?;
            return Err(MutationError::UpdateRejected { source: error });
        }
        Err(error) => tracing::warn!(
            "Immediate sync: Failed to update '{name}' (ID: {guardrail_id}) in memory: {error}"
        ),
    }
    // Prisma's update returns every column; the review columns come from a second read of the row
    let review = database
        .review_rows
        .find_unique(&guardrail_id)
        .await
        .map_err(MutationError::internal)?;
    let Some(review) = review else {
        return Err(MutationError::missing_after_update(&guardrail_id));
    };
    Ok(Json(StoredGuardrail::from_rows(updated, review)))
}
