use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_core::api::vector_store_access::VectorStoreAccess;
use betterlite_core::api::vector_store_access::auth::ObjectPermissionLookup;
use betterlite_core::api::vector_store_access::error::VectorStoreAccessError;
use betterlite_core::api::vector_store_access::store::ManagedVectorStore;
use betterlite_openai::api::vector_store_records::records::vector::LiteLlmManagedVectorStore;

use super::error::VectorManagementError;
use super::rows::{VectorStoreRows, row_to_vector_store};

// _check_vector_store_access: the record and the caller, handed to B1060's grant as they came
pub async fn check_vector_store_access<Permissions, Teams, SessionTeams>(
    access: &VectorStoreAccess<'_, Permissions, Teams, SessionTeams>,
    store: &LiteLlmManagedVectorStore,
    auth: &UserAPIKeyAuth,
) -> Result<bool, VectorStoreAccessError>
where
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    access
        .can_user_access_vector_store(&ManagedVectorStore(store.clone()), auth)
        .await
}

// _fetch_and_authorize_vector_store: 404 before any access check, then 403 when the grant refuses
pub async fn fetch_store_before_access<Rows, Permissions, Teams, SessionTeams>(
    rows: &Rows,
    access: &VectorStoreAccess<'_, Permissions, Teams, SessionTeams>,
    vector_store_id: &str,
    auth: &UserAPIKeyAuth,
) -> Result<LiteLlmManagedVectorStore, VectorManagementError>
where
    Rows: VectorStoreRows + ?Sized,
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    let Some(row) = rows.find_unique(vector_store_id).await? else {
        return Err(VectorManagementError::NotFound {
            vector_store_id: vector_store_id.to_owned(),
        });
    };
    let store = row_to_vector_store(row)?;
    if !check_vector_store_access(access, &store, auth).await? {
        return Err(VectorManagementError::AccessDenied);
    }
    Ok(store)
}
