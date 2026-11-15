use crate::agent_policy::AgentResponse;

use super::AgentRegistry;
use super::AgentRegistryApi as _;

mod tests;

pub(super) fn get_agent_list(
    registry: &AgentRegistry,
    agent_names: Option<&[String]>,
) -> Vec<AgentResponse> {
    let state = registry.state();
    let agents = state
        .agent_list
        .iter()
        .filter(|agent| agent_names.is_none_or(|names| names.contains(&agent.agent_name)));
    agents.cloned().collect()
}

pub(super) fn get_public_agent_list(
    registry: &AgentRegistry,
    public_agent_groups: Option<&[String]>,
) -> Vec<AgentResponse> {
    let Some(groups) = public_agent_groups else {
        return Vec::new();
    };
    let agents = registry.get_agent_list(None);
    agents
        .into_iter()
        .filter(|agent| {
            let ids = registry.ids_for_agent(&agent.agent_id);
            groups.iter().any(|group| ids.contains(group))
        })
        .collect()
}
