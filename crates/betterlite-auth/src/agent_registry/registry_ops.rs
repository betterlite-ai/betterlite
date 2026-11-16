use crate::agent_policy::AgentResponse;

use super::AgentRegistry;
use super::AgentRegistryApi as _;

mod tests;

pub(super) fn get_agent_list(
    registry: &AgentRegistry,
    agent_names: Option<&[String]>,
) -> Vec<AgentResponse> {
    let mut agents = registry.state().agent_list.clone();
    if let Some(names) = agent_names {
        agents.retain(|agent| names.contains(&agent.agent_name));
    }
    agents
}

pub(super) fn get_public_agent_list(
    registry: &AgentRegistry,
    groups: &[String],
) -> Vec<AgentResponse> {
    let mut public = registry.get_agent_list(None);
    public.retain(|agent| {
        let ids = registry.ids_for_agent(&agent.agent_id);
        groups.iter().any(|group| ids.contains(group))
    });
    public
}
