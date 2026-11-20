use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistryApi;

mod tests;

// B4776's registry behind the one lookup the limiter needs; a case can supply its own mapping
pub trait AgentPolicyRegistry: Send + Sync {
    #[must_use]
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse>;
}

impl<T: AgentRegistryApi + Send + Sync> AgentPolicyRegistry for T {
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.get_agent_by_id(agent_id)
    }
}
