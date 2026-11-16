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
use super::{MutationDatabase, MutationState, authenticated, require_admin};
use crate::auth::auth_identity::AuthContext;

const PENDING_REVIEW: &str = "pending_review";
const ACTIVE: &str = "active";
const REJECTED: &str = "rejected";
const APPROVED_MESSAGE: &str = "Guardrail approved";
const REJECTED_MESSAGE: &str = "Guardrail rejected";
const APPROVE_FAILED: &str = "Error approving guardrail submission";
const REJECT_FAILED: &str = "Error rejecting guardrail submission";

#[derive(Clone, Copy)]
pub(super) enum Decision {
    Approve,
    Reject,
}

pub(super) async fn review_submission<R, S>(
    State(state): State<Arc<MutationState<R, S>>>,
    Path(guardrail_id): Path<String>,
    Extension(decision): Extension<Decision>,
    caller: Option<Extension<AuthContext>>,
) -> Result<Json<SubmissionDecision>, MutationError>
where
    R: GuardrailRows + 'static,
    S: ReviewRows + 'static,
{
    let (failed, status, message) = match decision {
        Decision::Approve => (APPROVE_FAILED, ACTIVE, APPROVED_MESSAGE),
        Decision::Reject => (REJECT_FAILED, REJECTED, REJECTED_MESSAGE),
    };
    let (database, row) = pending_submission(&state, caller, &guardrail_id, failed).await?;
    database
        .review_rows
        .set_status(&guardrail_id, status, Utc::now())
        .await
        .map_err(|source| logged(failed, MutationError::ReviewRows { source }))?;
    let warning = match decision {
        Decision::Approve => start_in_memory(database, &row, &guardrail_id).await?,
        Decision::Reject => None,
    };
    Ok(Json(SubmissionDecision {
        guardrail_id,
        status,
        message,
        warning,
    }))
}

async fn start_in_memory<R, S>(
    database: &MutationDatabase<R, S>,
    row: &ReviewRow,
    guardrail_id: &str,
) -> Result<Option<String>, MutationError>
where
    R: GuardrailRows,
    S: ReviewRows,
{
    let Some(params) =
        parse_json_object(row.litellm_params.as_ref()).filter(|params| !params.0.is_empty())
    else {
        return Err(MutationError::MissingParams);
    };
    match start_approved(database, row, params).await {
        Ok(()) => {
            tracing::info!(
                "Approved guardrail {} (ID: {guardrail_id}) and initialized in memory",
                row.guardrail_name
            );
            Ok(None)
        }
        Err(error) => {
            tracing::warn!(
                "Failed to initialize approved guardrail {guardrail_id} in memory: {error}"
            );
            Ok(Some(format!(
                "Guardrail was marked active but failed to initialize in memory: {error}. It will be picked up on the next sync cycle."
            )))
        }
    }
}

async fn pending_submission<'state, R, S>(
    state: &'state MutationState<R, S>,
    caller: Option<Extension<AuthContext>>,
    guardrail_id: &str,
    context: &str,
) -> Result<(&'state MutationDatabase<R, S>, ReviewRow), MutationError>
where
    R: GuardrailRows,
    S: ReviewRows,
{
    let auth = authenticated(caller)?;
    require_admin(&auth, REVIEW_ADMIN_REQUIRED)?;
    let database = state.database.as_ref().ok_or(MutationError::NoDatabase)?;
    let row = database
        .review_rows
        .find_unique(guardrail_id)
        .await
        .map_err(|source| logged(context, MutationError::ReviewRows { source }))?
        .ok_or(MutationError::SubmissionNotFound)?;
    if row.status != PENDING_REVIEW {
        return Err(MutationError::NotPendingReview { status: row.status });
    }
    Ok((database, row))
}

// LiteLLM's _parse_json_field: an object as stored, or text that parses to an object
fn parse_json_object(value: Option<&JsonData>) -> Option<OpaqueParams> {
    let text = match value? {
        JsonData::Object(fields) => serde_json::to_string(fields).ok()?,
        JsonData::String(text) => text.clone(),
        JsonData::Null | JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) => {
            return None;
        }
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
