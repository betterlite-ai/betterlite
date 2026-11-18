use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use betterlite_db::resource_rows::credential::{
    CreateCredentialItem, CredentialBase, CredentialItem, CredentialItemFields,
};

use crate::auth::auth_identity::AuthContext;

use super::context::{actor, deployment_values};
use super::encryption::encrypt_credential_values;
use super::error::CredentialError;
use super::lookup::DeploymentLookup;
use super::request::decode_credential_request;
use super::state::CredentialState;
use super::store::CredentialStoreError;
use super::wire::SuccessResponse;

pub(super) async fn create_credential(
    State(state): State<CredentialState>,
    identity: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<SuccessResponse>, CredentialError> {
    let request = decode_credential_request::<CreateCredentialItem>(&body)?;
    let Some(store) = state.store.as_deref() else {
        return Err(CredentialError::DbNotConnected);
    };
    let credential = created_credential(request, state.lookup.as_deref())?;
    let encrypted = encrypt_credential_values(&state.cipher, &credential, None)?;
    let actor = actor(identity.as_ref());
    match store.create(&encrypted, actor.as_deref()).await {
        Ok(()) => {}
        Err(CredentialStoreError::Duplicate) => {
            return Err(CredentialError::Duplicate {
                credential_name: credential.fields.base.credential_name,
            });
        }
        Err(source) => return Err(CredentialError::Store { source }),
    }
    state.registry.upsert_credentials(vec![credential]);
    Ok(Json(SuccessResponse {
        success: true,
        message: "Credential created successfully",
    }))
}

fn created_credential(
    request: CreateCredentialItem,
    lookup: Option<&dyn DeploymentLookup>,
) -> Result<CredentialItem, CredentialError> {
    let model_id = request
        .fields
        .model_id
        .as_deref()
        .filter(|model_id| !model_id.is_empty());
    let credential_values = match model_id {
        Some(model_id) => deployment_values(lookup, model_id)?,
        None => request
            .fields
            .credential_values
            .ok_or(CredentialError::MissingValues)?,
    };
    Ok(CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: request.fields.base.credential_name,
                credential_info: request.fields.base.credential_info,
            },
            credential_values,
        },
    })
}
