use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};
use betterlite_config::sections::guardrail_params::routes::GuardrailInfoResponse;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_pyhost::json_values::JsonObject;

use super::caller::authenticated;
use super::error::ReadError;
use super::masking::mask_sensitive_values;
use super::projection::{config_dump, stored_dump, validated};
use super::state::ReadState;
use crate::auth::auth_identity::AuthContext;

pub(super) async fn guardrail_info<R>(
    caller: Option<Extension<AuthContext>>,
    State(state): State<Arc<ReadState<R>>>,
    Path(guardrail_id): Path<String>,
) -> Result<Json<GuardrailInfoResponse>, ReadError>
where
    R: GuardrailRows,
{
    authenticated(caller)?;
    if state.database
        && let Some(row) = state
            .runtime
            .registry()
            .get_guardrail_by_id_from_db(&guardrail_id)
            .await?
    {
        let dump = stored_dump(&row.litellm_params)?;
        let response = info_response(&row, &dump, GuardrailDefinitionLocation::Db)?;
        return Ok(Json(response));
    }
    // only a config-sourced entry: a db-backed one missing from the db is stale
    let found = {
        let handler = state.runtime.handler().read().await;
        handler.get_config_guardrail_by_id(&guardrail_id).cloned()
    };
    let Some(guardrail) = found else {
        return Err(ReadError::NotFound { guardrail_id });
    };
    let dump = config_dump(&guardrail.litellm_params)?;
    let response = info_response(&guardrail, &dump, GuardrailDefinitionLocation::Config)?;
    Ok(Json(response))
}

// LiteLLM writes guardrail_info as dict(x or {}), so an absent one becomes an empty object here
fn info_response(
    guardrail: &Guardrail,
    params: &JsonObject,
    location: GuardrailDefinitionLocation,
) -> Result<GuardrailInfoResponse, ReadError> {
    let masked = mask_sensitive_values(params);
    Ok(GuardrailInfoResponse {
        guardrail_id: guardrail.guardrail_id.clone(),
        guardrail_name: guardrail.guardrail_name.clone(),
        litellm_params: validated(&masked)?,
        guardrail_info: Some(guardrail.guardrail_info.clone().unwrap_or_default()),
        created_at: guardrail.created_at,
        updated_at: guardrail.updated_at,
        guardrail_definition_location: location,
    })
}
