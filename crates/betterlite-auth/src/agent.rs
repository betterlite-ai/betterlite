use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;

mod tests;

// The registry row a session-budget hook reads: the agent's id and its litellm_params.
#[derive(Clone, Debug, Default)]
pub struct AgentRef {
    pub agent_id: String,
    pub litellm_params: BTreeMap<String, Box<RawValue>>,
}

// AgentRegistry.get_agent_by_id: an exact agent_id match over the whole list first, then the translated legacy id.
#[must_use]
pub fn get_agent_by_id(registry: &AgentRegistry, id: &str) -> Option<AgentRef> {
    let agent = registry.get_agent_by_id(id)?;
    Some(AgentRef {
        agent_id: agent.agent_id,
        litellm_params: agent.litellm_params.unwrap_or_default(),
    })
}
