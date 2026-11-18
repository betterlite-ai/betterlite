use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Extension, Path, State};
use betterlite_config::credentials::{CredentialItem, CredentialValues};
use betterlite_db::resource_rows::credential::UpdateCredentialItem;

use crate::auth::auth_identity::AuthContext;

use super::context::{actor, deployment_values};
use super::error::CredentialError;
use super::lookup::DeploymentLookup;
use super::merge::{CredentialUpdate, update_db_credential};
use super::request::decode_credential_request;
use super::state::CredentialState;
use super::wire::SuccessResponse;

pub(super) async fn update_credential(
    State(state): State<CredentialState>,
    Path(credential_name): Path<String>,
    identity: Option<Extension<AuthContext>>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<SuccessResponse>, CredentialError> {
    let bytes = body.map_err(|source| CredentialError::ReadBody { source })?;
    let request = decode_credential_request::<UpdateCredentialItem>(&bytes)?;
    let Some(store) = state.store.as_deref() else {
        return Err(CredentialError::DbNotConnected);
    };
    let existing = store
        .find_by_name(&credential_name)
        .await?
        .ok_or(CredentialError::CredentialNotFoundInDb)?;
    let patch = credential_patch(request, state.lookup.as_deref())?;
    let actor = actor(identity.as_ref());
    let merged = update_db_credential(
        store,
        &state.cipher,
        &existing,
        CredentialUpdate {
            patch: &patch,
            new_encryption_key: None,
            actor: actor.as_deref(),
        },
    )
    .await?;
    state.registry.apply_credential_patch(
        &credential_name,
        &merged.fields.base.credential_name,
        &patch,
    );
    Ok(Json(SuccessResponse {
        success: true,
        message: "Credential updated successfully",
    }))
}

fn credential_patch(
    request: UpdateCredentialItem,
    lookup: Option<&dyn DeploymentLookup>,
) -> Result<CredentialItem, CredentialError> {
    let model_id = request
        .fields
        .model_id
        .as_deref()
        .filter(|model_id| !model_id.is_empty());
    let credential_values = match model_id {
        Some(deployment_id) => deployment_values(lookup, deployment_id)?,
        None => CredentialValues::from(request.fields.credential_values.unwrap_or_default()),
    };
    Ok(CredentialItem {
        credential_name: request.fields.credential_name,
        credential_info: request.fields.credential_info,
        credential_values,
    })
}
