use std::future::Future;

use betterlite_mcp::envelope::EnvelopeIdentity;
use betterlite_mcp::envelope::models::SubjectType;
use betterlite_mcp::gateway_dcr::ports::ReloadUserFailure;

use super::key_reload::ActiveKeyReload as _;
use super::user_reload::ActiveUserLoad as _;
use super::{ActiveSubjects, SubjectStore, UserRowSource};

mod tests;

// The workspace denies clippy::multiple_inherent_impl, so this module's part of the type's API lives on this trait.
pub trait SubjectRevalidation {
    fn revalidate_subject(
        &self,
        identity: &EnvelopeIdentity,
    ) -> impl Future<Output = Option<ReloadUserFailure>> + Send;
}

impl<S: SubjectStore> SubjectRevalidation for ActiveSubjects<S> {
    // None means the sealed subject is still live; a deactivated key owner is revoked like a dead key
    async fn revalidate_subject(&self, identity: &EnvelopeIdentity) -> Option<ReloadUserFailure> {
        match identity.subject_type {
            SubjectType::KeyHash => {
                let resolved = match self.reload_active_key(&identity.subject).await {
                    Ok(resolved) => resolved,
                    Err(failure) => return Some(failure),
                };
                if self.key_owner_deactivated(&resolved.key).await {
                    return Some(ReloadUserFailure::NoActiveKey);
                }
                None
            }
            SubjectType::UserId => self
                .load_active_user(&identity.subject, UserRowSource::Cache)
                .await
                .err(),
        }
    }
}
