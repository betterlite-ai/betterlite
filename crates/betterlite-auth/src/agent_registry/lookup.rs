use crate::agent_policy::AgentResponse;

use super::AgentRegistry;

mod tests;

pub(super) fn get_agent_by_id(registry: &AgentRegistry, agent_id: &str) -> Option<AgentResponse> {
    let state = registry.state();
    let by_key = |key: &str| state.agent_list.iter().find(|agent| agent.agent_id == key);
    let found = by_key(agent_id)
        .or_else(|| by_key(state.legacy_to_stable.get(agent_id)?.as_str()))
        .cloned();
    drop(state);
    found
}

pub(super) fn get_agent_by_name(
    registry: &AgentRegistry,
    agent_name: &str,
) -> Option<AgentResponse> {
    registry
        .state()
        .agent_list
        .iter()
        .find(|agent| agent.agent_name == agent_name)
        .cloned()
}
