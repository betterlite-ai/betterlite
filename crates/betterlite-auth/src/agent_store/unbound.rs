use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::{BindingKey, MICROSOFT_ENTRA_PROVIDER};

mod tests;

pub async fn unbound_client(
    store: &AgentIdentityStore,
    tenant_id: &str,
    client_id: &str,
) -> Result<(), AgentIdentityFailure> {
    let Some(retired) = &store.retired else {
        return Ok(());
    };
    let key = BindingKey {
        provider: MICROSOFT_ENTRA_PROVIDER,
        tenant_id,
        client_id,
    };
    let found = retired
        .find_unique(&key)
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
