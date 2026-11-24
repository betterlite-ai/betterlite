use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::ui_session_identity::UiSessionTeams;
use betterlite_auth::vector_index_admin::auth_is_proxy_admin;

use super::VectorStoreAccess;
use super::auth::ObjectPermissionLookup;
use super::error::VectorStoreAccessError;
use super::store::ManagedVectorStore;

impl<Permissions, Teams, SessionTeams> VectorStoreAccess<'_, Permissions, Teams, SessionTeams>
where
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    // can_user_access_vector_store: admin, then a teamless store, then any effective context's grant
    pub async fn can_user_access_vector_store(
        &self,
        vector_store: &ManagedVectorStore,
        auth: &UserAPIKeyAuth,
    ) -> Result<bool, VectorStoreAccessError> {
        if auth_is_proxy_admin(auth) {
            return Ok(true);
        }
        if vector_store.team_id_is_null_or_absent() {
            return Ok(true);
        }
        let auth_contexts = self.vector_store_auth_contexts(auth).await?;
        Ok(self
            .is_vector_store_granted_to_any(vector_store, &auth_contexts)
            .await)
    }

    // filter_listable_vector_stores: admins keep every store; anyone else keeps the granted ones in order
    pub async fn filter_listable_vector_stores<'store>(
        &self,
        vector_stores: &'store [ManagedVectorStore],
        auth: &UserAPIKeyAuth,
    ) -> Result<Vec<&'store ManagedVectorStore>, VectorStoreAccessError> {
        if auth_is_proxy_admin(auth) {
            return Ok(vector_stores.iter().collect());
        }
        let auth_contexts = self.vector_store_auth_contexts(auth).await?;
        let mut listable = Vec::new();
        for vector_store in vector_stores {
            if self
                .is_vector_store_granted_to_any(vector_store, &auth_contexts)
                .await
            {
                listable.push(vector_store);
            }
        }
        Ok(listable)
    }
}
