use super::cache::CachedText;
use super::errors::ContainerOwnershipError;
use super::repository::ContainerObjectTable;
use super::{
    CONTAINER_OBJECT_PURPOSE, ContainerForwardingParams, ContainerIdentity, ContainerOwnership,
    container_model_object_id,
};
use crate::api::response_id_codecs::container::decode_container_id;

// get_container_forwarding_params: the upstream id and provider, plus the deployment model when one is encoded
pub async fn forwarding_params<Rows>(
    ownership: &ContainerOwnership<Rows>,
    container_id: &str,
    identity: &ContainerIdentity,
) -> Result<ContainerForwardingParams, ContainerOwnershipError>
where
    Rows: ContainerObjectTable,
{
    let mut model_id = decode_container_id(container_id).model_id.flatten();
    if model_id.is_none() {
        let stored_id = stored_container_id(ownership, identity).await?;
        if let Some(recovered) = stored_id.filter(|stored| stored.as_str() != container_id) {
            model_id = decode_container_id(&recovered).model_id.flatten();
        }
    }
    Ok(ContainerForwardingParams {
        container_id: identity.original_container_id.clone(),
        custom_llm_provider: identity.custom_llm_provider.clone(),
        model_id,
    })
}

// _get_stored_container_id: the encoded id captured at create time, so a native id recovers its model
pub(crate) async fn stored_container_id<Rows>(
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
    match ownership.caches.stored_id.read(&model_object_id)? {
        CachedText::Absent => return Ok(None),
        CachedText::Known(stored_id) => return Ok(Some(stored_id)),
        CachedText::Miss => {}
    }
    if !ownership.database_ready()? {
        return Ok(None);
    }
    let stored_id = ownership
        .objects
        .find_row(&model_object_id, Some(CONTAINER_OBJECT_PURPOSE))
        .await?
        .and_then(|row| row.unified_object_id)
        .filter(|found| !found.is_empty());
    ownership
        .caches
        .stored_id
        .write(&model_object_id, stored_id.as_deref())?;
    Ok(stored_id)
}
