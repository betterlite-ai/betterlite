use axum::Json;
use axum::extract::{Path, State};

use super::error::CredentialError;
use super::state::CredentialState;
use super::wire::SuccessResponse;

pub(super) async fn delete_credential(
    State(state): State<CredentialState>,
    Path(credential_name): Path<String>,
) -> Result<Json<SuccessResponse>, CredentialError> {
    let Some(store) = state.store.as_deref() else {
        return Err(CredentialError::DbNotConnected);
    };
    if !store.delete(&credential_name).await? {
        return Err(CredentialError::CredentialNotFound { credential_name });
    }
    state.registry.remove_credential(&credential_name);
    Ok(Json(SuccessResponse {
        success: true,
        message: "Credential deleted successfully",
    }))
}
