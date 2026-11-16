use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_guardrails::guardrail_registry::error::RegistryError;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use chrono::Utc;

use super::error::{MutationError, REVIEW_ADMIN_REQUIRED, logged};
use super::response::SubmissionDecision;
use super::review_rows::{ReviewRow, ReviewRows};
use super::{MutationDatabase, MutationState, authenticated, mutation_database, require_admin};
use crate::auth::auth_identity::AuthContext;

const PENDING_REVIEW: &str = "pending_review";
const ACTIVE: &str = "active";
const REJECTED: &str = "rejected";
const APPROVED_MESSAGE: &str = "Guardrail approved";
const REJECTED_MESSAGE: &str = "Guardrail rejected";
const APPROVE_FAILED: &str = "Error approving guardrail submission";
const REJECT_FAILED: &str = "Error rejecting guardrail submission";

pub(super) async fn approve_guardrail_submission<R, S>(
    State(state): State<Arc<MutationState<R, S>>>,
    Path(guardrail_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<SubmissionDecision>, MutationError>
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    let context = authenticated(caller)?;
    require_admin(&context, REVIEW_ADMIN_REQUIRED)?;
    let database = mutation_database(&state)?;
    let row = pending_row(database, &guardrail_id, APPROVE_FAILED).await?;
    database
        .review_rows
        .set_status(&guardrail_id, ACTIVE, Utc::now())
        .await
        .map_err(|error| logged(APPROVE_FAILED, &error))?;
    let Some(params) =
        parse_json_object(row.litellm_params.as_ref()).filter(|params| !params.0.is_empty())
    else {
        return Err(MutationError::MissingParams);
    };
    match start_approved(database, &row, params).await {
        Ok(()) => {
            tracing::info!(
                "Approved guardrail {} (ID: {guardrail_id}) and initialized in memory",
                row.guardrail_name
            );
            Ok(Json(decision(guardrail_id, ACTIVE, APPROVED_MESSAGE, None)))
        }
        Err(error) => {
            tracing::warn!(
                "Failed to initialize approved guardrail {guardrail_id} in memory: {error}"
            );
            let warning = format!(
                "Guardrail was marked active but failed to initialize in memory: {error}. It will be picked up on the next sync cycle."
            );
            Ok(Json(decision(
                guardrail_id,
                ACTIVE,
                APPROVED_MESSAGE,
                Some(warning),
            )))
        }
    }
}

pub(super) async fn reject_guardrail_submission<R, S>(
    State(state): State<Arc<MutationState<R, S>>>,
    Path(guardrail_id): Path<String>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<SubmissionDecision>, MutationError>
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    let context = authenticated(caller)?;
    require_admin(&context, REVIEW_ADMIN_REQUIRED)?;
    let database = mutation_database(&state)?;
    pending_row(database, &guardrail_id, REJECT_FAILED).await?;
    database
        .review_rows
        .set_status(&guardrail_id, REJECTED, Utc::now())
        .await
        .map_err(|error| logged(REJECT_FAILED, &error))?;
    Ok(Json(decision(
        guardrail_id,
        REJECTED,
        REJECTED_MESSAGE,
        None,
    )))
}

fn decision(
    guardrail_id: String,
    status: &'static str,
    message: &'static str,
    warning: Option<String>,
) -> SubmissionDecision {
    SubmissionDecision {
        guardrail_id,
        status,
        message,
        warning,
    }
}

async fn pending_row<R, S>(
    database: &MutationDatabase<R, S>,
    guardrail_id: &str,
    context: &str,
) -> Result<ReviewRow, MutationError>
where
    R: GuardrailRows,
    S: ReviewRows,
{
    let row = database
        .review_rows
        .find_unique(guardrail_id)
        .await
        .map_err(|error| logged(context, &error))?;
    let Some(row) = row else {
        return Err(MutationError::SubmissionNotFound);
    };
    if row.status != PENDING_REVIEW {
        return Err(MutationError::NotPendingReview {
            status: row.status.clone(),
        });
    }
    Ok(row)
}

// LiteLLM's _parse_json_field: an object as stored, or text that parses to an object
fn parse_json_object(value: Option<&JsonData>) -> Option<OpaqueParams> {
    let text = match value? {
        JsonData::Object(fields) => serde_json::to_string(fields).ok()?,
        JsonData::String(text) => text.clone(),
        _ => return None,
    };
    serde_json::from_str(&text).ok()
}

async fn start_approved<R, S>(
    database: &MutationDatabase<R, S>,
    row: &ReviewRow,
    params: OpaqueParams,
) -> Result<(), RegistryError>
where
    R: GuardrailRows,
    S: ReviewRows,
{
    let bytes = serde_json::to_vec(&params).map_err(|source| RegistryError::Encode { source })?;
    let litellm_params = decode_litellm_params(&bytes).map_err(RegistryError::Params)?;
    let guardrail = Guardrail {
        guardrail_id: Some(row.guardrail_id.clone()),
        guardrail_name: row.guardrail_name.clone(),
        litellm_params,
        guardrail_info: Some(parse_json_object(row.guardrail_info.as_ref()).unwrap_or_default()),
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: row.team_id.clone(),
    };
    database
        .runtime
        .handler()
        .write()
        .await
        .initialize_guardrail(guardrail, GuardrailDefinitionLocation::Db)
        .map(|_initialized| ())
}
