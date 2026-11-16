use betterlite_models::decisions::Json;

use crate::agent_policy::AgentResponse;

use super::AgentRegistry;
use super::AgentRegistryApi as _;
use super::AgentRegistryError;

mod tests;

pub(super) fn load_agents_from_db_and_config(
    registry: &AgentRegistry,
    agent_config: Option<&[Json]>,
    db_agents: Option<&[AgentResponse]>,
) -> Result<Vec<AgentResponse>, AgentRegistryError> {
    registry.reset_agent_list();
    if let Some(agents) = db_agents {
        for agent in agents {
            registry.register_agent(agent.clone());
        }
    }
    let remembered: Option<Vec<Json>> = agent_config.is_none().then(|| {
        let state = registry.state();
        state.config_agents.clone()
    });
    let config = agent_config.or(remembered.as_deref());
    registry.load_agents_from_config(config)?;
    Ok(registry.get_agent_list(None))
}
