use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use betterlite_db::resource_rows::credential::{
    CredentialBase, CredentialItem, CredentialItemFields, UpdateCredentialItem,
};

use crate::auth::auth_identity::AuthContext;

use super::context::{actor, deployment_values};
use super::error::CredentialError;
use super::lookup::DeploymentLookup;
use super::merge::{CredentialUpdate, update_db_credential};
use super::registry::CredentialRegistry;
use super::request::decode_credential_request;
use super::state::CredentialState;
use super::wire::SuccessResponse;

pub(super) async fn update_credential(
    State(state): State<CredentialState>,
    Path(credential_name): Path<String>,
    identity: Option<Extension<AuthContext>>,
    body: Bytes,
) -> Result<Json<SuccessResponse>, CredentialError> {
    let request = decode_credential_request::<UpdateCredentialItem>(&body)?;
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
    sync_memory(&state.registry, &credential_name, &patch, &merged);
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
        Some(model_id) => deployment_values(lookup, model_id)?,
        None => request.fields.credential_values.unwrap_or_default(),
    };
    Ok(CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: request.fields.credential_name,
                credential_info: request.fields.credential_info,
            },
            credential_values,
        },
    })
}

fn sync_memory(
    registry: &CredentialRegistry,
    credential_name: &str,
    patch: &CredentialItem,
    merged: &CredentialItem,
) {
    let Some(existing) = registry.find_credential(credential_name) else {
        return;
    };
    let mut values = existing.fields.credential_values;
    if !patch.fields.credential_values.0.is_empty() {
        values.0.extend(patch.fields.credential_values.0.clone());
    }
    let mut info = existing.fields.base.credential_info;
    if !patch.fields.base.credential_info.0.is_empty() {
        info.0.extend(patch.fields.base.credential_info.0.clone());
    }
    let new_name = &merged.fields.base.credential_name;
    if new_name != credential_name {
        registry.remove_credential(credential_name);
    }
    registry.upsert_credentials(vec![CredentialItem {
        fields: CredentialItemFields {
            base: CredentialBase {
                credential_name: new_name.clone(),
                credential_info: info,
            },
            credential_values: values,
        },
    }]);
}
