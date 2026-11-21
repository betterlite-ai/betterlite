use std::collections::BTreeSet;

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;

mod tests;

// An empty restricted set denies every agent
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentAccess {
    Unrestricted,
    Restricted(BTreeSet<String>),
}

#[must_use]
pub fn to_stable_ids(registry: &AgentRegistry, ids: &BTreeSet<String>) -> BTreeSet<String> {
    let mut stable_ids = BTreeSet::new();
    for agent_id in ids {
        stable_ids.insert(registry.stable_agent_id(agent_id));
    }
    stable_ids
}

// Both sides map to stable ids first, so a grant stored under a legacy config id still matches
#[must_use]
pub fn intersect_agent_access(
    registry: &AgentRegistry,
    key_access: &AgentAccess,
    team_access: &AgentAccess,
) -> AgentAccess {
    match (key_access, team_access) {
        (AgentAccess::Unrestricted, AgentAccess::Unrestricted) => AgentAccess::Unrestricted,
        (AgentAccess::Restricted(ids), AgentAccess::Unrestricted)
        | (AgentAccess::Unrestricted, AgentAccess::Restricted(ids)) => {
            AgentAccess::Restricted(to_stable_ids(registry, ids))
        }
        (AgentAccess::Restricted(key_ids), AgentAccess::Restricted(team_ids)) => {
            let team_stable = to_stable_ids(registry, team_ids);
            AgentAccess::Restricted(
                to_stable_ids(registry, key_ids)
                    .into_iter()
                    .filter(|agent_id| team_stable.contains(agent_id))
                    .collect(),
            )
        }
    }
}
