use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use betterlite_db::identity_rows::Payload;

use super::context::resolve_deployment;
use super::error::CredentialError;
use super::masking::{masked_credential, masked_values};
use super::state::CredentialState;
use super::wire::{CredentialListResponse, CredentialView};

pub(super) async fn list_credentials(
    State(state): State<CredentialState>,
) -> Json<CredentialListResponse> {
    let credentials = state
        .registry
        .credential_list()
        .iter()
        .map(masked_credential)
        .collect();
    Json(CredentialListResponse {
        success: true,
        credentials,
    })
}

pub(super) async fn get_credential_by_name(
    State(state): State<CredentialState>,
    Path(credential_name): Path<String>,
) -> Result<Json<CredentialView>, CredentialError> {
    match state.registry.find_credential(&credential_name) {
        Some(credential) => Ok(Json(masked_credential(&credential))),
        None => Err(CredentialError::CredentialNotFound { credential_name }),
    }
}

pub(super) async fn get_credential_by_model(
    State(state): State<CredentialState>,
    Path(model_id): Path<String>,
) -> Result<Json<CredentialView>, CredentialError> {
    let deployment = resolve_deployment(
        state.lookup.as_deref(),
        &model_id,
        CredentialError::RouterMissing,
    )?;
    let credential_values = masked_values(&deployment.credential_values);
    let credential_name = format!("{}-credential-{model_id}", deployment.model_name);
    Ok(Json(CredentialView {
        credential_name,
        credential_values,
        credential_info: Payload(BTreeMap::new()),
    }))
}
