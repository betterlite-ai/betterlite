#![cfg(test)]

use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_LEGACY, ALPHA_STABLE, BETA_ENTRY, BETA_STABLE, agent, loaded,
};

fn names(agents: &[AgentResponse]) -> Vec<String> {
    agents
        .iter()
        .map(|agent| agent.agent_name.clone())
        .collect()
}

fn ids(agents: &[AgentResponse]) -> Vec<String> {
    agents.iter().map(|agent| agent.agent_id.clone()).collect()
}

#[test]
fn register_agent_allows_duplicates() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("a1", "same"));
    registry.register_agent(agent("a2", "same"));
    let agents = registry.get_agent_list(None);
    assert_eq!(ids(&agents), vec![String::from("a1"), String::from("a2")]);
    assert_eq!(registry.get_agent_by_name("same").unwrap().agent_id, "a1");
}

#[test]
fn deregister_removes_every_agent_with_the_name() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("a1", "alpha"));
    registry.register_agent(agent("b1", "b\u{e9}ta"));
    registry.register_agent(agent("a2", "alpha"));
    registry.deregister_agent("alpha");
    assert_eq!(
        names(&registry.get_agent_list(None)),
        vec![String::from("b\u{e9}ta")]
    );
    registry.deregister_agent("absent");
    assert_eq!(registry.get_agent_list(None).len(), 1);
}

#[test]
fn get_agent_list_filters_by_name_in_list_order() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("b1", "b\u{e9}ta"));
    registry.register_agent(agent("a1", "alpha"));
    let wanted = vec![String::from("alpha"), String::from("b\u{e9}ta")];
    assert_eq!(
        ids(&registry.get_agent_list(Some(&wanted))),
        vec![String::from("b1"), String::from("a1")]
    );
    assert!(registry.get_agent_list(Some(&[])).is_empty());
    assert_eq!(registry.get_agent_list(None).len(), 2);
}

#[test]
fn get_public_agent_list_needs_groups() {
    let registry = loaded(&[ALPHA_ENTRY]);
    assert!(registry.get_public_agent_list(None).is_empty());
    assert!(registry.get_public_agent_list(Some(&[])).is_empty());
    assert!(
        registry
            .get_public_agent_list(Some(&[String::from("unknown")]))
            .is_empty()
    );
}

#[test]
fn get_public_agent_list_matches_stable_and_legacy_ids() {
    let registry = loaded(&[ALPHA_ENTRY, BETA_ENTRY]);
    let stable = vec![String::from(ALPHA_STABLE)];
    assert_eq!(ids(&registry.get_public_agent_list(Some(&stable))), stable);
    let aliases = vec![String::from(ALPHA_LEGACY), String::from(BETA_STABLE)];
    assert_eq!(
        ids(&registry.get_public_agent_list(Some(&aliases))),
        vec![String::from(ALPHA_STABLE), String::from(BETA_STABLE)]
    );
}

#[test]
fn get_public_agent_list_keeps_agents_registered_without_config() {
    let registry = loaded(&[ALPHA_ENTRY]);
    registry.register_agent(agent("db-zeta", "zeta"));
    let groups = vec![String::from(ALPHA_LEGACY), String::from("db-zeta")];
    assert_eq!(
        ids(&registry.get_public_agent_list(Some(&groups))),
        vec![String::from(ALPHA_STABLE), String::from("db-zeta")]
    );
}
