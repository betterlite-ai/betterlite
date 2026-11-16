use super::cache::CachedOwner;
use super::errors::ContainerOwnershipError;
use super::repository::ContainerObjectTable;
use super::{
    ContainerCaller, ContainerIdentity, ContainerOwnership, container_model_object_id,
    decode_for_ownership,
};
use crate::api::managed_resource_store::{is_proxy_admin, user_can_access_resource_owner};

// assert_user_can_access_container: an admin bypasses the row, everyone else needs the owner scope
pub async fn assert_container_access<Rows>(
    ownership: &ContainerOwnership<Rows>,
    container_id: &str,
    caller: ContainerCaller<'_>,
) -> Result<ContainerIdentity, ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let identity = decode_for_ownership(container_id, caller.custom_llm_provider);
    if is_proxy_admin(caller.principal) {
        return Ok(identity);
    }
    let owner = container_owner(ownership, &identity).await?;
    if !user_can_access_resource_owner(owner.as_deref(), caller.principal) {
        return Err(ContainerOwnershipError::Forbidden);
    }
    Ok(identity)
}

pub(crate) async fn container_owner<Rows>(
    ownership: &ContainerOwnership<Rows>,
    identity: &ContainerIdentity,
) -> Result<Option<String>, ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let model_object_id = container_model_object_id(
        &identity.original_container_id,
        &identity.custom_llm_provider,
    );
    match ownership.caches.owner(&model_object_id)? {
        CachedOwner::Untracked => return Ok(None),
        CachedOwner::Owned(owner) => return Ok(Some(owner)),
        CachedOwner::Miss => {}
    }
    if !ownership.database_ready()? {
        return Ok(None);
    }
    let row = ownership.objects.find_container(&model_object_id).await?;
    let owner = row.as_ref().and_then(|found| found.created_by.clone());
    let stored_id = row
        .as_ref()
        .and_then(|found| found.unified_object_id.as_deref());
    ownership
        .caches
        .store_owner(&model_object_id, owner.as_deref())?;
    ownership
        .caches
        .store_stored_id(&model_object_id, stored_id)?;
    Ok(owner)
}
