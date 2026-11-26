use crate::agent_policy::AgentResponse;

// AgentLookup: the registered-agent lookups a JWT agent claim is matched against.
pub trait AgentLookup: Send + Sync {
    fn get_agent_list(&self) -> Vec<AgentResponse>;
    fn get_agent_by_id(&self, agent_id: &str) -> Option<AgentResponse>;
    fn get_agent_by_name(&self, agent_name: &str) -> Option<AgentResponse>;
}

// _NoRegisteredAgents: the lookup in force until the proxy binds its registry, so no claim matches.
pub struct NoRegisteredAgents;

impl AgentLookup for NoRegisteredAgents {
    fn get_agent_list(&self) -> Vec<AgentResponse> {
        Vec::new()
    }

    fn get_agent_by_id(&self, _agent_id: &str) -> Option<AgentResponse> {
        None
    }

    fn get_agent_by_name(&self, _agent_name: &str) -> Option<AgentResponse> {
        None
    }
}
