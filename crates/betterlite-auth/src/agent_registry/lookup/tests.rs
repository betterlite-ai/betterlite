#![cfg(test)]

use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_LEGACY, ALPHA_STABLE, agent, entries, loaded,
};

#[test]
fn an_exact_id_wins_over_the_legacy_translation() {
    let registry = loaded(&[ALPHA_ENTRY]);
    registry.register_agent(agent(ALPHA_LEGACY, "shadow"));
    let exact = registry.get_agent_by_id(ALPHA_LEGACY).unwrap();
    assert_eq!(exact.agent_name, "shadow");
    let stable = registry.get_agent_by_id(ALPHA_STABLE).unwrap();
    assert_eq!(stable.agent_name, "alpha");
}

#[test]
fn a_legacy_id_resolves_to_its_agent() {
    let registry = loaded(&[ALPHA_ENTRY]);
    let found = registry.get_agent_by_id(ALPHA_LEGACY).unwrap();
    assert_eq!(found.agent_id, ALPHA_STABLE);
    assert_eq!(found.agent_name, "alpha");
}

#[test]
fn unknown_ids_return_none() {
    let registry = loaded(&[ALPHA_ENTRY]);
    assert!(registry.get_agent_by_id("unknown").is_none());
    assert!(registry.get_agent_by_name("unknown").is_none());
}

#[test]
fn get_agent_by_name_returns_the_first_match() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("first", "same"));
    registry.register_agent(agent("second", "same"));
    assert_eq!(
        registry.get_agent_by_name("same").unwrap().agent_id,
        "first"
    );
}

#[test]
fn a_shadowed_config_entry_has_no_agent_to_resolve_to() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("db-alpha", "alpha"));
    let config = entries(&[ALPHA_ENTRY]);
    registry.load_agents_from_config(Some(&config)).unwrap();
    assert!(registry.get_agent_by_id(ALPHA_LEGACY).is_none());
    assert_eq!(
        registry.get_agent_by_id("db-alpha").unwrap().agent_name,
        "alpha"
    );
}
