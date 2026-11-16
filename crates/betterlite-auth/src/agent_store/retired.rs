use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;

mod tests;

pub async fn agent_is_retired(
    store: &AgentIdentityStore,
    agent_id: &str,
) -> Result<bool, AgentIdentityFailure> {
    let Some(retired_agents) = &store.retired_agents else {
        return Err(AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Agent history is unavailable"),
        });
    };
    retired_agents
        .find_unique(agent_id)
        .await
        .map(|row| row.is_some())
        .map_err(|_source| AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Agent history is unavailable"),
        })
}
