use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::MICROSOFT_ENTRA_PROVIDER;

mod tests;

impl AgentIdentityStore {
    pub async fn unbound_client(
        &self,
        tenant_id: &str,
        client_id: &str,
    ) -> Result<(), AgentIdentityFailure> {
        let Some(retired) = &self.retired else {
            return Ok(());
        };
        let found = retired
            .find_unique(MICROSOFT_ENTRA_PROVIDER, tenant_id, client_id)
            .await
            .map_err(|_source| AgentIdentityFailure {
                code: FailureCode::PolicyUnavailable,
                message: String::from("Retired agent identity could not be checked"),
            })?;
        if found.is_some() {
            return Err(AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from("This agent identity binding has been retired"),
            });
        }
        Ok(())
    }
}
