use crate::agent_policy::ManagedAgentContext;
use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;
use super::records::{AgentStateFilter, AuthenticationFilter};

mod tests;

pub async fn record_authentication(
    store: &AgentIdentityStore,
    context: &ManagedAgentContext,
) -> Result<(), AgentIdentityFailure> {
    let Some(revision) = context.binding_revision.as_deref() else {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Agent authentication requires a binding revision"),
        });
    };
    let filter = AuthenticationFilter {
        agent_id: &context.agent_id,
        revision,
        active: true,
        agent: AgentStateFilter {
            enabled: true,
            identity_managed: true,
        },
    };
    let count = store
        .identities
        .update_many(&filter, (store.clock)())
        .await
        .map_err(|_source| AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Agent authentication could not be recorded"),
        })?;
    if count == 1 {
        return Ok(());
    }
    Err(AgentIdentityFailure {
        code: FailureCode::IdentityDenied,
        message: String::from("Agent identity changed during authentication; retry"),
    })
}
