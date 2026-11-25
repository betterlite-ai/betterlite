use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::ui_session_identity::UiSessionTeams;

use crate::api::vector_store_access::VectorStoreAccess;
use crate::api::vector_store_access::auth::ObjectPermissionLookup;
use crate::api::vector_store_access::store::ManagedVectorStore;

use super::error::{ACCESS_DENIED_DETAIL, ManagedStoreAccessError, ManagedStorePolicyError};
use super::lookup::{ManagedStoreLookup, ManagedStoreRegistry, get_managed_vector_store};

// assert_user_can_access_vector_store: 403 with the caller's detail unless B1060's policy allows the store
pub async fn assert_can_access_vector_store<Permissions, Teams, SessionTeams>(
    access: &VectorStoreAccess<'_, Permissions, Teams, SessionTeams>,
    vector_store: &ManagedVectorStore,
    auth: &UserAPIKeyAuth,
    detail: Option<&str>,
) -> Result<(), ManagedStorePolicyError>
where
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    if access
        .can_user_access_vector_store(vector_store, auth)
        .await?
    {
        return Ok(());
    }
    let denial_detail = detail.unwrap_or(ACCESS_DENIED_DETAIL);
    Err(ManagedStorePolicyError::Denied {
        detail: String::from(denial_detail),
    })
}

// assert_user_can_access_vector_store_id: only a managed store is checked, so an unknown id passes
pub async fn assert_can_access_store_id<Registry, Permissions, Teams, SessionTeams>(
    lookup: &ManagedStoreLookup<'_, Registry>,
    access: &VectorStoreAccess<'_, Permissions, Teams, SessionTeams>,
    vector_store_id: &str,
    auth: &UserAPIKeyAuth,
    detail: Option<&str>,
) -> Result<Option<ManagedVectorStore>, ManagedStoreAccessError<Registry::Error>>
where
    Registry: ManagedStoreRegistry,
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    let store = get_managed_vector_store(lookup, vector_store_id).await?;
    let Some(vector_store) = &store else {
        return Ok(None);
    };
    assert_can_access_vector_store(access, vector_store, auth, detail).await?;
    Ok(store)
}
