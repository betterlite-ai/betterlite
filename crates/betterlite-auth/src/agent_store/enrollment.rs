use crate::identity::{AgentIdentityFailure, FailureCode, MicrosoftInteractiveSubject};

use super::AgentIdentityStore;
use super::records::{HUMAN_KIND, InteractiveEnrollment, SSO_INTERACTIVE};

mod tests;

impl AgentIdentityStore {
    pub async fn enroll_interactive_human(
        &self,
        subject: &MicrosoftInteractiveSubject,
        user_id: &str,
    ) -> Result<(), AgentIdentityFailure> {
        let enrollment = InteractiveEnrollment {
            issuer: &subject.issuer,
            tenant_id: &subject.tenant_id,
            oid: &subject.oid,
            user_id,
            verified_via: SSO_INTERACTIVE,
        };
        let row =
            self.humans
                .upsert(&enrollment)
                .await
                .map_err(|_source| AgentIdentityFailure {
                    code: FailureCode::PolicyUnavailable,
                    message: String::from("Microsoft subject enrollment is unavailable"),
                })?;
        let bound_elsewhere = row.kind != HUMAN_KIND
            || row.verified_via != SSO_INTERACTIVE
            || row.user_id.as_deref() != Some(user_id);
        if bound_elsewhere {
            return Err(AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from(
                    "Microsoft subject is already bound to another local identity",
                ),
            });
        }
        Ok(())
    }
}
