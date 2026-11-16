use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_config::sections::guardrail_params::base::BaseLitellmParams;
use betterlite_config::sections::guardrail_params::error::GuardrailParamsError;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use betterlite_config::sections::guardrail_params::routes::PatchGuardrailRequest;
use betterlite_guardrails::guardrail_registry::error::RegistryError;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_guardrails::guardrail_startup::reconcile::GuardrailReconcile as _;
use chrono::Utc;

use super::body::decode_patch_body;
use super::error::{MANAGE_ADMIN_REQUIRED, MutationError, is_configuration_error, logged};
use super::response::StoredGuardrail;
use super::review_rows::ReviewRows;
use super::{MutationState, authenticated, require_admin};
use crate::auth::auth_identity::AuthContext;

const PATCH_FAILED: &str = "Error updating guardrail";

pub(super) async fn patch_guardrail<R, S>(
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
    let request = decode_patch_body(&body).map_err(|error| MutationError::Body(Box::new(error)))?;
    require_admin(&context, MANAGE_ADMIN_REQUIRED)?;
    let database = state.database.as_ref().ok_or(MutationError::NoDatabase)?;
    let registry = database.runtime.registry();
    let existing = registry
        .get_guardrail_by_id_from_db(&guardrail_id)
        .await
        .map_err(|source| logged(PATCH_FAILED, MutationError::Registry { source }))?;
    let Some(existing) = existing else {
        return Err(MutationError::NotFound { guardrail_id });
    };
    let merged = merge_guardrail(&guardrail_id, &existing, &request)?;
    let updated = registry
        .update_guardrail_in_db(&guardrail_id, &merged, Utc::now())
        .await
        .map_err(|source| logged(PATCH_FAILED, MutationError::Registry { source }))?;
    let name = updated.guardrail_name.clone();
    let synced = database
        .runtime
        .handler()
        .write()
        .await
        .sync_guardrail_from_db(merged);
    match synced {
        Ok(_held) => tracing::info!(
            "Immediate sync: Successfully updated guardrail '{name}' (ID: {guardrail_id})"
        ),
        Err(error) => {
            if is_configuration_error(&error) {
                registry
                    .update_guardrail_in_db(&guardrail_id, &existing, Utc::now())
                    .await
                    .map_err(|source| logged(PATCH_FAILED, MutationError::Registry { source }))?;
                return Err(MutationError::UpdateRejected { source: error });
            }
            tracing::warn!(
                "Immediate sync: Failed to update '{name}' (ID: {guardrail_id}) in memory: {error}"
            );
        }
    }
    let review = database
        .review_rows
        .find_unique(&guardrail_id)
        .await
        .map_err(|source| logged(PATCH_FAILED, MutationError::ReviewRows { source }))?;
    let Some(review) = review else {
        return Err(MutationError::MissingAfterUpdate { guardrail_id });
    };
    Ok(Json(StoredGuardrail::from_rows(updated, review)))
}

fn merge_guardrail(
    guardrail_id: &str,
    existing: &Guardrail,
    request: &PatchGuardrailRequest,
) -> Result<Guardrail, MutationError> {
    let params = match request.litellm_params.as_ref() {
        Some(sent) => merge_params(&existing.litellm_params, sent).map_err(|source| {
            MutationError::UpdateRejected {
                source: RegistryError::Params(source),
            }
        })?,
        None => existing.litellm_params.clone(),
    };
    let info = match request.guardrail_info.clone() {
        Some(info) => Some(info),
        None => existing.guardrail_info.clone(),
    };
    Ok(Guardrail {
        guardrail_id: Some(guardrail_id.to_owned()),
        guardrail_name: request
            .guardrail_name
            .clone()
            .unwrap_or_else(|| existing.guardrail_name.clone()),
        litellm_params: params,
        guardrail_info: info,
        policy_template: None,
        created_at: None,
        updated_at: None,
        team_id: None,
    })
}

// pydantic's exclude_unset: only the keys the client sent overwrite the stored ones
fn merge_params(
    stored: &LitellmParams,
    sent: &BaseLitellmParams,
) -> Result<LitellmParams, GuardrailParamsError> {
    let overrides = sent.sent_fields()?;
    let encoded =
        serde_json::to_vec(stored).map_err(|source| GuardrailParamsError::Encode { source })?;
    let mut merged = serde_json::from_slice::<OpaqueParams>(&encoded)
        .map_err(|source| GuardrailParamsError::Encode { source })?
        .0;
    merged.extend(overrides);
    LitellmParams::try_from(OpaqueParams(merged))
}
