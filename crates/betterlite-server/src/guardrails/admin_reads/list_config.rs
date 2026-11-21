use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use betterlite_config::sections::guardrail_params::base::BaseLitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailDefinitionLocation;
use betterlite_config::sections::guardrail_params::routes::{
    GuardrailInfoResponse, ListGuardrailsResponse,
};
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;

use super::caller::authenticated;
use super::error::ReadError;
use super::masking::mask_sensitive_values;
use super::projection::opaque_params;
use super::state::{ConfigGuardrail, ReadState};
use crate::auth::auth_identity::AuthContext;

// the raw guardrails: list of the loaded config file, never the database
pub(super) async fn list_guardrails<R>(
    caller: Option<Extension<AuthContext>>,
    State(state): State<Arc<ReadState<R>>>,
) -> Result<Json<ListGuardrailsResponse>, ReadError>
where
    R: GuardrailRows,
{
    authenticated(caller)?;
    let guardrails = state.config.guardrails();
    let mut response = ListGuardrailsResponse {
        guardrails: Vec::with_capacity(guardrails.len()),
    };
    for guardrail in &guardrails {
        response.guardrails.push(list_entry(guardrail)?);
    }
    Ok(Json(response))
}

fn list_entry(guardrail: &ConfigGuardrail) -> Result<GuardrailInfoResponse, ReadError> {
    let params = guardrail.litellm_params.clone().unwrap_or_default();
    let masked = mask_sensitive_values(&params);
    Ok(GuardrailInfoResponse {
        guardrail_id: guardrail.guardrail_id.clone(),
        guardrail_name: guardrail.guardrail_name.clone(),
        // LiteLLM validates even the empty dict, which yields the all-null default model
        litellm_params: Some(BaseLitellmParams::from_map(opaque_params(&masked)?.0)?),
        guardrail_info: guardrail.guardrail_info.clone(),
        created_at: None,
        updated_at: None,
        guardrail_definition_location: GuardrailDefinitionLocation::Config,
    })
}
