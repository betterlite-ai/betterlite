use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::subject_state::{active_key_user_id, user_scim_deactivated};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_mcp::gateway_dcr::ports::{ReloadUser, ReloadUserFailure};
use std::future::Future;

use super::{ActiveSubjects, SubjectReadError, SubjectStore, UserRowSource};

mod tests;

impl<S: SubjectStore> ActiveSubjects<S> {
    pub async fn load_active_user(
        &self,
        user_id: &str,
        source: UserRowSource,
    ) -> Result<LiteLLMUserTable, ReloadUserFailure> {
        if !self.store.has_database() {
            return Err(ReloadUserFailure::Unresolvable);
        }
        let user = match self.store.user_by_id(user_id, source).await {
            Ok(Some(user)) => user,
            Ok(None) => return Err(ReloadUserFailure::NoActiveKey),
            Err(SubjectReadError::Refused) => return Err(ReloadUserFailure::NoActiveKey),
            Err(SubjectReadError::Unavailable) => return Err(ReloadUserFailure::Unavailable),
            Err(SubjectReadError::Faulted) => return Err(ReloadUserFailure::Faulted),
            Err(SubjectReadError::Other) => {
                tracing::debug!("_reload_active_user_by_id: user-resolution error");
                return Err(ReloadUserFailure::NoActiveKey);
            }
        };
        if user_scim_deactivated(&user) {
            return Err(ReloadUserFailure::NoActiveKey);
        }
        Ok(user)
    }

    // Fail open: a missing owner record or a database blip must not revoke a live key
    pub async fn key_owner_deactivated(&self, key: &UserAPIKeyAuth) -> bool {
        let Some(owner) = active_key_user_id(key, self.store.now()) else {
            return false;
        };
        if !self.store.has_database() {
            return false;
        }
        match self.store.user_by_id(owner, UserRowSource::Cache).await {
            Ok(Some(user)) => user_scim_deactivated(&user),
            Ok(None) => false,
            Err(error) => {
                tracing::debug!(
                    ?error,
                    "refresh: key-owner SCIM lookup failed, not revoking"
                );
                false
            }
        }
    }
}

impl<S: SubjectStore> ReloadUser for ActiveSubjects<S> {
    fn reload_user(&self, user_id: &str) -> impl Future<Output = Option<ReloadUserFailure>> + Send {
        async move {
            self.load_active_user(user_id, UserRowSource::Cache)
                .await
                .err()
        }
    }
}
