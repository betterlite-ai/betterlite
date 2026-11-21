use std::collections::BTreeSet;

use crate::agent_policy::AgentResponse;

mod tests;

// LiteLLM's getattr(agent, "agent_access_groups", None) finds nothing: AgentResponse drops the agents table column, so the config side never matches a declared group
pub(crate) const fn config_agent_ids(
    _agents: &[AgentResponse],
    _groups: &[String],
) -> BTreeSet<String> {
    BTreeSet::new()
}
