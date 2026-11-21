use std::future::Future;

use betterlite_auth::agent_identity::error::{
    AgentIdentityError, IDENTITY_DENIED_STATUS, raise_identity_failure,
};
use betterlite_auth::agent_store::AgentIdentityStore;
use betterlite_auth::agent_store::enrollment as identity_enrollment;
use betterlite_auth::identity::{AgentIdentityFailure, MicrosoftInteractiveSubject};

pub trait InteractiveEnroller {
    fn enroll_interactive_human(
        &self,
        subject: &MicrosoftInteractiveSubject,
        user_id: &str,
    ) -> impl Future<Output = Result<(), AgentIdentityFailure>> + Send;
}

impl InteractiveEnroller for AgentIdentityStore {
    fn enroll_interactive_human(
        &self,
        subject: &MicrosoftInteractiveSubject,
        user_id: &str,
    ) -> impl Future<Output = Result<(), AgentIdentityFailure>> + Send {
        identity_enrollment::enroll_interactive_human(self, subject, user_id)
    }
}

// enroll_microsoft_subject silently skips a subject or user id it cannot trust, and a store failure is the caller's error
pub async fn enroll_microsoft_subject<E>(
    subject: Option<&MicrosoftInteractiveSubject>,
    user_id: Option<&str>,
    store: &E,
) -> Result<(), AgentIdentityError>
where
    E: InteractiveEnroller,
{
    let Some(subject) = subject else {
        return Ok(());
    };
    let Some(user_id) = user_id.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    store
        .enroll_interactive_human(subject, user_id)
        .await
        .map_err(|failure| raise_identity_failure(failure, IDENTITY_DENIED_STATUS))
}
