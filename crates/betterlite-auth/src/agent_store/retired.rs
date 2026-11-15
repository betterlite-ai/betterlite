use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;

mod tests;

impl AgentIdentityStore {
    pub async fn retired_agent(&self, agent_id: &str) -> Result<bool, AgentIdentityFailure> {
        let Some(retired_agents) = &self.retired_agents else {
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
}
