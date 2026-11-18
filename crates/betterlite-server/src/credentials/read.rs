use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, State};
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem, CredentialItemFields,
};

use super::error::CredentialError;
use super::masking::{mask_fields, masked_credential};
use super::state::CredentialState;
use super::wire::CredentialListResponse;

pub(super) async fn list_credentials(
    State(state): State<CredentialState>,
) -> Json<CredentialListResponse> {
    let credentials = state
        .registry
        .credential_list()
        .into_iter()
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
) -> Result<Json<CredentialItem>, CredentialError> {
    let credential = state
        .registry
        .find_credential(&credential_name)
        .ok_or(CredentialError::CredentialNotFound { credential_name })?;
    Ok(Json(masked_credential(credential)))
}

pub(super) async fn get_credential_by_model(
    State(state): State<CredentialState>,
    Path(model_id): Path<String>,
) -> Result<Json<CredentialItem>, CredentialError> {
    let Some(lookup) = state.lookup.as_deref() else {
        return Err(CredentialError::RouterMissing);
    };
    let deployment = lookup
        .deployment_credentials(&model_id)
        .ok_or(CredentialError::ModelNotFound)?;
    Ok(Json(CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: format!("{}-credential-{model_id}", deployment.model_name),
                credential_info: Payload(BTreeMap::new()),
            },
            credential_values: Payload(mask_fields(&deployment.credential_values.0, 0)),
        },
    }))
}
