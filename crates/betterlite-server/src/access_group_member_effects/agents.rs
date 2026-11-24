use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};

use crate::access_group_read::protocols::tables::{AccessGroupTx, AgentTable as _};

use super::error::{MemberEffectsError, RegistrationRejected};

// B4776's registry, as the two lookups and two mutations this module makes
pub trait AgentRegistryEffects: Send + Sync {
    fn find(&self, agent_id: &str) -> Option<AgentResponse>;

    fn deregister(&self, agent_name: &str);

    fn register(&self, agent: AgentResponse) -> Result<(), RegistrationRejected>;
}

impl AgentRegistryEffects for AgentRegistry {
    fn find(&self, agent_id: &str) -> Option<AgentResponse> {
        self.get_agent_by_id(agent_id)
    }

    fn deregister(&self, agent_name: &str) {
        self.deregister_agent(agent_name);
    }

    // the registry always accepts a registration; the injected port is what can reject one
    fn register(&self, agent: AgentResponse) -> Result<(), RegistrationRejected> {
        self.register_agent(agent);
        Ok(())
    }
}

// _without_access_group: null reads as no groups, and every exact occurrence goes
#[must_use]
pub fn without_access_group(groups: Option<&[String]>, access_group_id: &str) -> Vec<String> {
    groups
        .unwrap_or_default()
        .iter()
        .filter(|id| id.as_str() != access_group_id)
        .cloned()
        .collect()
}

pub async fn detach_group_from_agents<Tx>(
    tx: &Tx,
    access_group_id: &str,
) -> Result<Vec<String>, MemberEffectsError>
where
    Tx: AccessGroupTx + ?Sized,
{
    let agents = tx
        .litellm_agentstable()
        .find_carrying(access_group_id)
        .await?;
    let mut detached = Vec::with_capacity(agents.len());
    for agent in &agents {
        let groups = without_access_group(agent.access_group_ids.as_deref(), access_group_id);
        tx.litellm_agentstable()
            .set_access_group_ids(&agent.agent_id, &groups)
            .await?;
        detached.push(agent.agent_id.clone());
    }
    Ok(detached)
}

// the lookup happens for every id before any mutation, and a rejected register stops the rest
pub fn detach_group_from_registry<Registry>(
    registry: &Registry,
    agent_ids: &[String],
    access_group_id: &str,
) -> Result<(), RegistrationRejected>
where
    Registry: AgentRegistryEffects + ?Sized,
{
    let registered: Vec<AgentResponse> = agent_ids
        .iter()
        .filter_map(|agent_id| registry.find(agent_id))
        .collect();
    for agent in registered {
        let groups = without_access_group(agent.access_group_ids.as_deref(), access_group_id);
        registry.deregister(&agent.agent_name);
        let patched = AgentResponse {
            access_group_ids: Some(groups),
            ..agent
        };
        registry.register(patched)?;
    }
    Ok(())
}
