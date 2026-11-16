use betterlite_auth::auth_identity::records::Principal;

use super::cache::allowed_ids_cache_key;
use super::errors::ContainerOwnershipError;
use super::repository::{ContainerObjectTable, ContainerOwnerUpdate, ContainerOwnerWrite};
use super::response::{owner_file_object, response_provider, synthetic_container};
use super::{
    CONTAINER_OBJECT_PURPOSE, ContainerCaller, ContainerOwnership, container_model_object_id,
    decode_for_ownership, owner_scope_strings,
};
use crate::api::managed_resource_store::{
    OwnerScope, get_primary_owner_scope, user_can_access_resource_owner,
};
use crate::api::response_id_traversal::container::collect_container_ids;
use crate::api::response_id_traversal::record::Record;

pub async fn record_container_owner<Rows>(
    ownership: &ContainerOwnership<Rows>,
    response: &Record,
    caller: ContainerCaller<'_>,
) -> Result<(), ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let Some(container_id) = response.field("id").and_then(Record::as_text) else {
        tracing::warn!("Skipping container ownership tracking because provider response has no id");
        return Ok(());
    };
    let Some(owner) = get_primary_owner_scope(caller.principal) else {
        return Err(ContainerOwnershipError::NoIdentityScope);
    };
    let (model_object_id, write) = owner_write(response, caller, container_id, &owner);
    if !ownership.database_ready()? {
        tracing::warn!("Skipping container ownership tracking because prisma_client is None");
        return Ok(());
    }
    store_owned_container(ownership, write, caller.principal).await?;
    ownership
        .caches
        .store_owner(&model_object_id, Some(&owner.0))?;
    ownership
        .caches
        .store_stored_id(&model_object_id, Some(container_id))?;
    forget_allowed(ownership, caller.principal)
}

// the row record_container_owner writes: the decoded identity and the dumped response with it stamped on
fn owner_write(
    response: &Record,
    caller: ContainerCaller<'_>,
    container_id: &str,
    owner: &OwnerScope,
) -> (String, ContainerOwnerWrite) {
    let identity = decode_for_ownership(container_id, caller.custom_llm_provider);
    let model_object_id = container_model_object_id(
        &identity.original_container_id,
        &identity.custom_llm_provider,
    );
    let write = ContainerOwnerWrite {
        unified_object_id: container_id.to_owned(),
        model_object_id: model_object_id.clone(),
        file_object: owner_file_object(response, &identity),
        file_purpose: CONTAINER_OBJECT_PURPOSE.to_owned(),
        created_by: owner.0.clone(),
        updated_by: owner.0.clone(),
    };
    (model_object_id, write)
}

// the caller's own list page is dropped so the just-recorded container shows up on the next list
fn forget_allowed<Rows>(
    ownership: &ContainerOwnership<Rows>,
    principal: Option<&Principal>,
) -> Result<(), ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let scopes = owner_scope_strings(principal);
    if scopes.is_empty() {
        return Ok(());
    }
    ownership
        .caches
        .forget_allowed(&allowed_ids_cache_key(&scopes)?);
    Ok(())
}

// record_container_owners_from_responses_response: one bad container never stops the ones after it
pub async fn record_response_containers<Rows>(
    ownership: &ContainerOwnership<Rows>,
    response: &Record,
    principal: Option<&Principal>,
    custom_llm_provider: Option<&str>,
) where
    Rows: ContainerObjectTable,
{
    let container_ids = collect_container_ids(Some(response));
    if container_ids.is_empty() {
        return;
    }
    let resolved = custom_llm_provider
        .filter(|provider| !provider.is_empty())
        .map_or_else(|| response_provider(response), str::to_owned);
    let caller = ContainerCaller {
        principal,
        custom_llm_provider: &resolved,
    };
    for container_id in container_ids {
        let container = synthetic_container(&container_id);
        if let Err(error) = record_container_owner(ownership, &container, caller).await {
            tracing::error!(
                container_id = %container_id,
                error = %error,
                "Failed to record container ownership from responses output"
            );
        }
    }
}

pub(crate) async fn store_owned_container<Rows>(
    ownership: &ContainerOwnership<Rows>,
    write: ContainerOwnerWrite,
    principal: Option<&Principal>,
) -> Result<(), ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let Some(existing) = ownership
        .objects
        .find_unique(&write.model_object_id)
        .await?
    else {
        ownership.objects.create(write).await?;
        return Ok(());
    };
    if existing.file_purpose != CONTAINER_OBJECT_PURPOSE {
        return Err(ContainerOwnershipError::ConflictingPurpose);
    }
    if !user_can_access_resource_owner(existing.created_by.as_deref(), principal) {
        return Err(ContainerOwnershipError::Forbidden);
    }
    ownership
        .objects
        .update(ContainerOwnerUpdate {
            model_object_id: write.model_object_id,
            unified_object_id: write.unified_object_id,
            file_object: write.file_object,
            updated_by: write.updated_by,
        })
        .await?;
    Ok(())
}
