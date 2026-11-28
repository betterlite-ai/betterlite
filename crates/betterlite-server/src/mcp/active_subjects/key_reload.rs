use std::future::Future;

use betterlite_auth::master_key_admin::master_key_admin_auth;
use betterlite_auth::subject_state::key_is_active;
use betterlite_mcp::gateway_dcr::ports::ReloadUserFailure;
use betterlite_models::key_digest::hashing::hash_token;
use http::HeaderMap;

use super::presented_key::litellm_key_from_request;
use super::{ActiveSubjects, ResolvedKey, SubjectReadError, SubjectStore};

mod tests;

// The workspace denies clippy::multiple_inherent_impl, so this module's part of the type's API lives on this trait.
pub trait ActiveKeyReload {
    fn resolve_active_key(
        &self,
        headers: &HeaderMap,
    ) -> impl Future<Output = Result<ResolvedKey, ReloadUserFailure>> + Send;

    fn reload_active_key(
        &self,
        key_hash: &str,
    ) -> impl Future<Output = Result<ResolvedKey, ReloadUserFailure>> + Send;
}

impl<S: SubjectStore> ActiveKeyReload for ActiveSubjects<S> {
    async fn resolve_active_key(
        &self,
        headers: &HeaderMap,
    ) -> Result<ResolvedKey, ReloadUserFailure> {
        let Some(token) = litellm_key_from_request(headers) else {
            return Err(ReloadUserFailure::NoActiveKey);
        };
        self.reload_active_key(&hash_token(&token)).await
    }

    async fn reload_active_key(&self, key_hash: &str) -> Result<ResolvedKey, ReloadUserFailure> {
        match master_key_admin_auth(
            key_hash,
            self.settings.master_key.as_ref(),
            &self.settings.admin_name,
        ) {
            Ok(Some(key)) => {
                return Ok(ResolvedKey {
                    key_hash: key_hash.to_owned(),
                    key,
                });
            }
            Ok(None) => {}
            Err(_error) => return Err(ReloadUserFailure::Unresolvable),
        }
        if !self.store.has_database() {
            return Err(ReloadUserFailure::Unresolvable);
        }
        let key = match self.store.key_by_hash(key_hash).await {
            Ok(key) => key,
            Err(SubjectReadError::Refused) => return Err(ReloadUserFailure::NoActiveKey),
            Err(SubjectReadError::Unavailable) => return Err(ReloadUserFailure::Unavailable),
            Err(SubjectReadError::Faulted) => return Err(ReloadUserFailure::Faulted),
            Err(SubjectReadError::Other) => {
                tracing::debug!("_reload_active_key_by_hash: unexpected key-resolution error");
                return Err(ReloadUserFailure::Unresolvable);
            }
        };
        if !key_is_active(&key, self.store.now()) {
            return Err(ReloadUserFailure::NoActiveKey);
        }
        Ok(ResolvedKey {
            key_hash: key_hash.to_owned(),
            key,
        })
    }
}
