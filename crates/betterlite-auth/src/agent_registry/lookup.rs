use crate::agent_policy::AgentResponse;

use super::AgentRegistry;

mod tests;

pub(super) fn get_agent_by_id(registry: &AgentRegistry, agent_id: &str) -> Option<AgentResponse> {
    let state = registry.state();
    if let Some(found) = state
        .agent_list
        .iter()
        .find(|agent| agent.agent_id == agent_id)
    {
        return Some(found.clone());
    }
    let Some(translated) = state.legacy_to_stable.get(agent_id) else {
        return None;
    };
    state
        .agent_list
        .iter()
        .find(|agent| agent.agent_id == *translated)
        .cloned()
}

pub(super) fn get_agent_by_name(
    registry: &AgentRegistry,
    agent_name: &str,
) -> Option<AgentResponse> {
    let state = registry.state();
    let found = state
        .agent_list
        .iter()
        .find(|agent| agent.agent_name == agent_name);
    found.cloned()
}
