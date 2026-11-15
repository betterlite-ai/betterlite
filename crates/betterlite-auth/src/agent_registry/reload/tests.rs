#![cfg(test)]

use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::AgentRegistryError;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_STABLE, BETA_ENTRY, BETA_STABLE, agent, entries, loaded,
};

fn names(agents: &[AgentResponse]) -> Vec<String> {
    agents
        .iter()
        .map(|agent| agent.agent_name.clone())
        .collect()
}

#[test]
fn database_agents_register_before_config_entries() {
    let registry = AgentRegistry::default();
    let db = vec![agent("db-alpha", "alpha")];
    let config = entries(&[ALPHA_ENTRY, BETA_ENTRY]);
    let result = registry
        .load_agents_from_db_and_config(Some(&config), Some(&db))
        .unwrap();
    assert_eq!(
        names(&result),
        vec![String::from("alpha"), String::from("béta")]
    );
    assert_eq!(result[0].agent_id, "db-alpha");
    assert_eq!(result[1].agent_id, BETA_STABLE);
    assert_eq!(registry.ids_for_agent(ALPHA_STABLE).len(), 2);
}

#[test]
fn a_database_only_reload_keeps_the_remembered_config_agents() {
    let registry = loaded(&[ALPHA_ENTRY]);
    registry.register_agent(agent("db-beta", "béta"));
    let result = registry.load_agents_from_db_and_config(None, None).unwrap();
    assert_eq!(names(&result), vec![String::from("alpha")]);
    assert_eq!(result[0].agent_id, ALPHA_STABLE);
}

#[test]
fn an_explicit_config_becomes_the_remembered_one() {
    let registry = loaded(&[ALPHA_ENTRY]);
    let config = entries(&[BETA_ENTRY]);
    registry
        .load_agents_from_db_and_config(Some(&config), None)
        .unwrap();
    let rebuilt = registry.load_agents_from_db_and_config(None, None).unwrap();
    assert_eq!(names(&rebuilt), vec![String::from("béta")]);
    assert_eq!(rebuilt[0].agent_id, BETA_STABLE);
}

#[test]
fn an_error_half_way_leaves_the_list_as_it_was_at_that_moment() {
    let registry = AgentRegistry::default();
    let db = vec![agent("db-alpha", "alpha")];
    let config = entries(&[
        BETA_ENTRY,
        r#"{"agent_name":"gamma","agent_card_params":{"x":1},"agent_id":"given"}"#,
    ]);
    let error = registry
        .load_agents_from_db_and_config(Some(&config), Some(&db))
        .unwrap_err();
    assert!(matches!(error, AgentRegistryError::ConfigAgentId));
    let agents = registry.get_agent_list(None);
    assert_eq!(
        names(&agents),
        vec![String::from("alpha"), String::from("béta")]
    );
    assert_eq!(agents[0].agent_id, "db-alpha");
    assert_eq!(agents[1].agent_id, BETA_STABLE);
}
