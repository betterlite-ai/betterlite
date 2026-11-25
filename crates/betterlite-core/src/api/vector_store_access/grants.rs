use betterlite_auth::auth_team_lookup::lookup::{TeamRequest, TeamSource};
use betterlite_auth::identity_grants::loaders::TeamLoader;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::ui_session_identity::{
    UiSessionTeams, is_ui_session_credential, resolve_ui_session_team_ids,
};
use betterlite_auth::vector_index_admin::auth_is_proxy_admin;
use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;

use super::VectorStoreAccess;
use super::auth::{AuthContext, ObjectPermissionLookup};
use super::error::VectorStoreAccessError;
use super::store::ManagedVectorStore;

// _object_permission_allows_vector_store: only a record that lists the id explicitly allows it
#[must_use]
pub(super) fn permission_lists_vector_store(
    object_permission: Option<&LiteLLMObjectPermissionTable>,
    vector_store_id: &str,
) -> bool {
    let Some(permission) = object_permission else {
        return false;
    };
    let Some(allowed) = permission.vector_stores.0.as_deref() else {
        return false;
    };
    allowed
        .iter()
        .any(|entry| entry.as_str() == vector_store_id)
}

impl<Permissions, Teams, SessionTeams> VectorStoreAccess<'_, Permissions, Teams, SessionTeams>
where
    Permissions: ObjectPermissionLookup + ?Sized,
    Teams: TeamLoader + ?Sized,
    SessionTeams: UiSessionTeams + ?Sized,
{
    // _is_vector_store_granted: the key's grant, then the team's, then the store's own team
    pub async fn is_vector_store_granted(
        &self,
        vector_store: &ManagedVectorStore,
        context: &AuthContext,
    ) -> bool {
        let vector_store_id = vector_store.0.vector_store_id.as_deref().unwrap_or("");
        if self
            .level_allows(
                context.object_permission.as_ref(),
                context.object_permission_id.as_deref(),
                vector_store_id,
            )
            .await
        {
            return true;
        }
        if self
            .level_allows(
                context.team_object_permission.as_ref(),
                context.team_object_permission_id.as_deref(),
                vector_store_id,
            )
            .await
        {
            return true;
        }
        context.team_id.is_some() && context.team_id == vector_store.0.team_id
    }

    // _is_vector_store_granted_to_any: the first context that grants answers
    pub async fn is_vector_store_granted_to_any(
        &self,
        vector_store: &ManagedVectorStore,
        auth_contexts: &[AuthContext],
    ) -> bool {
        for context in auth_contexts {
            if self.is_vector_store_granted(vector_store, context).await {
                return true;
            }
        }
        false
    }

    // _vector_store_auth_contexts: one context per credential, or a session's key context plus its teams
    pub async fn vector_store_auth_contexts(
        &self,
        auth: &UserAPIKeyAuth,
    ) -> Result<Vec<AuthContext>, VectorStoreAccessError> {
        let base = AuthContext::from_auth(auth)?;
        if !is_ui_session_credential(auth) {
            return Ok(vec![base]);
        }
        let mut session_context = base.clone();
        session_context.without_team();
        let team_ids = resolve_ui_session_team_ids(auth, self.session_teams).await;
        let mut contexts = Vec::with_capacity(team_ids.len().saturating_add(1));
        contexts.push(session_context);
        for team_id in &team_ids {
            contexts.push(self.team_auth_context(&base, team_id).await?);
        }
        Ok(contexts)
    }

    // _team_auth_context: the credential with the loaded row's team fields
    pub async fn team_auth_context(
        &self,
        context: &AuthContext,
        team_id: &str,
    ) -> Result<AuthContext, VectorStoreAccessError> {
        let team = self
            .teams
            .load_team(TeamRequest {
                team_id,
                source: TeamSource::CacheThenDatabase,
                team_id_upsert: false,
            })
            .await?;
        let mut updated = context.clone();
        updated.with_team(team_id, &team);
        Ok(updated)
    }

    // _get_object_permission_for_id: no id or no database means no record; a failed load logs and grants nothing
    async fn get_object_permission_for_id(
        &self,
        object_permission_id: Option<&str>,
    ) -> Option<LiteLLMObjectPermissionTable> {
        let id = object_permission_id?;
        if id.is_empty() || !self.permissions.has_database() {
            return None;
        }
        match self.permissions.load_permission(id).await {
            Ok(permission) => permission,
            Err(error) => {
                tracing::debug!("Failed to load object_permission id={id}: {error}");
                None
            }
        }
    }

    // one grant level: an inline record, even an empty one, hides the row its id names
    async fn level_allows(
        &self,
        inline: Option<&LiteLLMObjectPermissionTable>,
        object_permission_id: Option<&str>,
        vector_store_id: &str,
    ) -> bool {
        if let Some(permission) = inline {
            return permission_lists_vector_store(Some(permission), vector_store_id);
        }
        let loaded = self
            .get_object_permission_for_id(object_permission_id)
            .await;
        permission_lists_vector_store(loaded.as_ref(), vector_store_id)
    }

    // can_user_access_vector_store: admin, then a teamless store, then any effective context's grant
    pub async fn can_user_access_vector_store(
        &self,
        vector_store: &ManagedVectorStore,
        auth: &UserAPIKeyAuth,
    ) -> Result<bool, VectorStoreAccessError> {
        if auth_is_proxy_admin(auth) {
            return Ok(true);
        }
        // the row keeps a team id of another JSON type in extra, and LiteLLM reads that id as a team
        let stored_team = vector_store
            .0
            .extra
            .get("team_id")
            .is_some_and(|value| !value.is_null());
        if vector_store.0.team_id.is_none() && !stored_team {
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
