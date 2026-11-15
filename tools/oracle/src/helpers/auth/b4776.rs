use std::collections::BTreeMap;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::agent_registry::AgentRegistryApi as _;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::OrHelperError as _;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    config: Option<Vec<Json>>,
    reload: bool,
    reload_config: Option<Vec<Json>>,
    db_agents: Vec<AgentResponse>,
    public_groups: Option<Vec<String>>,
    ids_for: Vec<String>,
    stable_ids: Vec<String>,
    by_id: Vec<String>,
    by_name: Vec<String>,
    deregister: Vec<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let registry = AgentRegistry::default();
        registry
            .load_agents_from_config(input.config.as_deref())
            .or_value_error()?;
        if input.reload {
            registry
                .load_agents_from_db_and_config(
                    input.reload_config.as_deref(),
                    Some(&input.db_agents),
                )
                .or_value_error()?;
        }
        let agents = snapshots(&registry.get_agent_list(None));
        let ids = ids_for(&registry, &input.ids_for);
        let stable = stable_ids(&registry, &input.stable_ids);
        let found_by_id = lookups_by_id(&registry, &input.by_id);
        let found_by_name = lookups_by_name(&registry, &input.by_name);
        let public = snapshots(&registry.get_public_agent_list(input.public_groups.as_deref()));
        for agent_name in &input.deregister {
            registry.deregister_agent(agent_name);
        }
        let after_deregister = snapshots(&registry.get_agent_list(None));
        Ok(json!({
            "agents": agents,
            "ids_for": ids,
            "stable_ids": stable,
            "by_id": found_by_id,
            "by_name": found_by_name,
            "public": public,
            "after_deregister": after_deregister,
        }))
    }
}

fn snapshot(agent: &AgentResponse) -> Value {
    json!({
        "agent_id": &agent.agent_id,
        "agent_name": &agent.agent_name,
    })
}

fn snapshots(agents: &[AgentResponse]) -> Vec<Value> {
    let mut values = Vec::with_capacity(agents.len());
    for agent in agents {
        values.push(snapshot(agent));
    }
    values
}

fn lookup(agent: Option<AgentResponse>) -> Value {
    match agent {
        Some(agent) => snapshot(&agent),
        None => Value::Null,
    }
}

fn ids_for(registry: &AgentRegistry, agent_ids: &[String]) -> BTreeMap<String, Vec<String>> {
    agent_ids
        .iter()
        .map(|agent_id| {
            let ids = registry.ids_for_agent(agent_id).into_iter().collect();
            (agent_id.clone(), ids)
        })
        .collect()
}

fn stable_ids(registry: &AgentRegistry, agent_ids: &[String]) -> Vec<String> {
    agent_ids
        .iter()
        .map(|agent_id| registry.stable_agent_id(agent_id))
        .collect()
}

fn lookups_by_id(registry: &AgentRegistry, agent_ids: &[String]) -> BTreeMap<String, Value> {
    agent_ids
        .iter()
        .map(|agent_id| {
            let found = lookup(registry.get_agent_by_id(agent_id));
            (agent_id.clone(), found)
        })
        .collect()
}

fn lookups_by_name(registry: &AgentRegistry, agent_names: &[String]) -> BTreeMap<String, Value> {
    agent_names
        .iter()
        .map(|agent_name| {
            let found = lookup(registry.get_agent_by_name(agent_name));
            (agent_name.clone(), found)
        })
        .collect()
}
