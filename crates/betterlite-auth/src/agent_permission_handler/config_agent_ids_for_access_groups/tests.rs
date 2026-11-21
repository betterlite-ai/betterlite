#![cfg(test)]

use std::collections::BTreeSet;

use super::config_agent_ids;
use crate::agent_permission_handler::tests::support::{registry, strings};
use crate::agent_registry::AgentRegistryApi as _;

#[test]
fn config_agents_never_match_a_declared_group() {
    let agents = registry(&[r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#]);
    let list = agents.get_agent_list(None);
    assert_eq!(config_agent_ids(&list, &strings(&["g1"])), BTreeSet::new());
}

#[test]
fn an_empty_group_list_returns_nothing() {
    let agents = registry(&[r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#]);
    let list = agents.get_agent_list(None);
    assert!(config_agent_ids(&list, &[]).is_empty());
}

#[test]
fn an_empty_registry_returns_nothing() {
    assert!(config_agent_ids(&[], &strings(&["g1"])).is_empty());
}
