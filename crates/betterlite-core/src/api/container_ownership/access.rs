use super::cache::CachedText;
use super::errors::ContainerOwnershipError;
use super::repository::ContainerObjectTable;
use super::{
    CONTAINER_OBJECT_PURPOSE, ContainerCaller, ContainerIdentity, ContainerOwnership,
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
    let model_object_id = format!(
        "{CONTAINER_OBJECT_PURPOSE}:{}:{}",
        identity.custom_llm_provider, identity.original_container_id
    );
    match ownership.caches.owner.read(&model_object_id)? {
        CachedText::Absent => return Ok(None),
        CachedText::Known(owner) => return Ok(Some(owner)),
        CachedText::Miss => {}
    }
    if !ownership.database_ready()? {
        return Ok(None);
    }
    let row = ownership
        .objects
        .find_row(&model_object_id, Some(CONTAINER_OBJECT_PURPOSE))
        .await?;
    let owner = row.as_ref().and_then(|found| found.created_by.clone());
    let stored_id = row
        .as_ref()
        .and_then(|found| found.unified_object_id.as_deref());
    ownership
        .caches
        .owner
        .write(&model_object_id, owner.as_deref())?;
    ownership
        .caches
        .stored_id
        .write(&model_object_id, stored_id)?;
    Ok(owner)
}
