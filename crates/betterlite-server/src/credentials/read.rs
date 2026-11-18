use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use betterlite_db::identity_rows::Payload;

use super::error::CredentialError;
use super::lookup::DeploymentLookup;
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
    let credential = state
        .registry
        .find_credential(&credential_name)
        .ok_or(CredentialError::CredentialNotFound { credential_name })?;
    Ok(Json(masked_credential(&credential)))
}

pub(super) async fn get_credential_by_model(
    State(state): State<CredentialState>,
    Path(model_id): Path<String>,
) -> Result<Json<CredentialView>, CredentialError> {
    deployment_view(state.lookup.as_deref(), &model_id).map(Json)
}

fn deployment_view(
    lookup: Option<&dyn DeploymentLookup>,
    model_id: &str,
) -> Result<CredentialView, CredentialError> {
    let Some(source) = lookup else {
        return Err(CredentialError::RouterMissing);
    };
    let deployment = source
        .deployment_credentials(model_id)
        .ok_or(CredentialError::ModelNotFound)?;
    Ok(CredentialView {
        credential_name: format!("{}-credential-{model_id}", deployment.model_name),
        credential_values: masked_values(&deployment.credential_values),
        credential_info: Payload(BTreeMap::new()),
    })
}
