use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use betterlite_config::sections::guardrail_params::records::GuardrailDefinitionLocation;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use chrono::Utc;

use super::body::{WriteRequest, decode_write_request};
use super::error::{MANAGE_ADMIN_REQUIRED, MutationError, is_configuration_error, logged};
use super::response::CreatedGuardrail;
use super::review_rows::ReviewRows;
use super::{MutationDatabase, MutationState, authenticated, mutation_database, require_admin};
use crate::auth::auth_identity::AuthContext;

const DB_FAILED: &str = "Error adding guardrail to db";

pub(super) async fn create_guardrail<R, S>(
    State(state): State<Arc<MutationState<R, S>>>,
    caller: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<CreatedGuardrail>, MutationError>
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    let context = authenticated(caller)?;
    let request = decode_write_request(&body).map_err(MutationError::body)?;
    require_admin(&context, MANAGE_ADMIN_REQUIRED)?;
    let database = mutation_database(&state)?;
    let WriteRequest {
        guardrail,
        guardrail_info,
    } = request;
    let created = database
        .runtime
        .registry()
        .add_guardrail_to_db(&guardrail, Utc::now())
        .await
        .map_err(|error| logged(DB_FAILED, &error))?;
    let guardrail_id = created.guardrail_id.clone().unwrap_or_default();
    let name = created.guardrail_name.clone();
    let started = database
        .runtime
        .handler()
        .write()
        .await
        .initialize_guardrail(created, GuardrailDefinitionLocation::Db);
    match started {
        Ok(_initialized) => tracing::info!(
            "Immediate sync: Successfully initialized guardrail '{name}' (ID: {guardrail_id})"
        ),
        Err(error) if is_configuration_error(&error) => {
            rollback(database, &guardrail_id).await;
            return Err(MutationError::Configuration { source: error });
        }
        Err(error) => tracing::warn!(
            "Immediate sync: Failed to initialize guardrail '{name}' (ID: {guardrail_id}) in memory: {error}"
        ),
    }
    Ok(Json(CreatedGuardrail {
        guardrail_id,
        guardrail_name: guardrail.guardrail_name,
        litellm_params: guardrail.litellm_params,
        guardrail_info,
        policy_template: guardrail.policy_template,
        created_at: guardrail.created_at,
        updated_at: guardrail.updated_at,
    }))
}

async fn rollback<R, S>(database: &MutationDatabase<R, S>, guardrail_id: &str)
where
    R: GuardrailRows,
    S: ReviewRows,
{
    let deleted = database
        .runtime
        .registry()
        .delete_guardrail_from_db(guardrail_id)
        .await;
    if let Err(error) = deleted {
        tracing::warn!("Rollback failed for guardrail '{guardrail_id}': {error}");
    }
}
