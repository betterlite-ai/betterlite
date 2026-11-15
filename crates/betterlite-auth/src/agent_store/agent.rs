use crate::agent_policy::AgentResponse;
use crate::identity::{AgentIdentityFailure, FailureCode};

use super::AgentIdentityStore;

mod tests;

impl AgentIdentityStore {
    pub async fn agent(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentResponse>, AgentIdentityFailure> {
        self.agents
            .find_unique(agent_id)
            .await
            .map_err(|_source| AgentIdentityFailure {
                code: FailureCode::PolicyUnavailable,
                message: String::from("Agent policy could not be loaded"),
            })
    }
}
