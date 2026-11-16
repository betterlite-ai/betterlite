#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::json;

use super::{AgentRef, get_agent_by_id};
use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;

fn config_json(agent_name: &str) -> Json {
    serde_json::from_value(json!({
        "agent_name": agent_name,
        "agent_card_params": {"model": "gpt-4o"},
        "litellm_params": {"max_budget_per_session": 1.5},
    }))
    .expect("the config entry is a JSON object")
}

fn registry_with_config(agent_name: &str) -> AgentRegistry {
    let registry = AgentRegistry::default();
    registry
        .load_agents_from_config(Some(&[config_json(agent_name)]))
        .expect("the config entry registers");
    registry
}

fn registered_agent(record: &str) -> AgentResponse {
    serde_json::from_str(record).expect("the registered agent is a valid record")
}

fn stable_agent(registry: &AgentRegistry) -> AgentResponse {
    registry
        .get_agent_list(None)
        .first()
        .cloned()
        .expect("the config agent is registered")
}

fn legacy_id_of(registry: &AgentRegistry, stable_id: &str) -> String {
    registry
        .ids_for_agent(stable_id)
        .into_iter()
        .find(|id| id != stable_id)
        .expect("a config agent carries a legacy id")
}

fn param<'agent>(agent: &'agent AgentRef, key: &str) -> Option<&'agent str> {
    agent.litellm_params.get(key).map(|raw| raw.get())
}

#[test]
fn exact_id_returns_the_agent() {
    let registry = AgentRegistry::default();
    registry.register_agent(registered_agent(
        r#"{"agent_id":"db-1","agent_name":"beta","agent_card_params":{"model":"gpt-4o"},"litellm_params":{"max_budget_per_session":1.5}}"#,
    ));
    let found = get_agent_by_id(&registry, "db-1").expect("the agent is registered under its id");
    assert_eq!(found.agent_id, "db-1");
    assert_eq!(param(&found, "max_budget_per_session"), Some("1.5"));
}

#[test]
fn legacy_id_resolves_to_the_stable_agent() {
    let registry = registry_with_config("alpha");
    let stable = stable_agent(&registry);
    let legacy = legacy_id_of(&registry, &stable.agent_id);
    let found = get_agent_by_id(&registry, &legacy).expect("the legacy id resolves");
    assert_eq!(found.agent_id, stable.agent_id);
    assert_eq!(param(&found, "max_budget_per_session"), Some("1.5"));
}

#[rstest]
#[case::unknown("missing")]
#[case::empty("")]
fn missing_id_returns_none(#[case] id: &str) {
    let registry = registry_with_config("alpha");
    assert!(get_agent_by_id(&registry, id).is_none());
}

#[test]
fn exact_id_wins_over_legacy_translation() {
    let probe = registry_with_config("alpha");
    let stable = stable_agent(&probe);
    let legacy = legacy_id_of(&probe, &stable.agent_id);
    let registry = AgentRegistry::default();
    let shadow = format!(
        r#"{{"agent_id":"{legacy}","agent_name":"shadow","agent_card_params":{{"model":"gpt-4o"}},"litellm_params":{{"shadow":true}}}}"#
    );
    registry.register_agent(registered_agent(&shadow));
    registry
        .load_agents_from_config(Some(&[config_json("alpha")]))
        .expect("the config entry registers");
    let found = get_agent_by_id(&registry, &legacy).expect("the exact id matches the shadow agent");
    assert_eq!(found.agent_id, legacy);
    assert_eq!(param(&found, "shadow"), Some("true"));
    assert_eq!(param(&found, "max_budget_per_session"), None);
}

#[test]
fn agent_without_params_has_an_empty_map() {
    let registry = AgentRegistry::default();
    registry.register_agent(registered_agent(
        r#"{"agent_id":"db-2","agent_name":"gamma","agent_card_params":{"model":"gpt-4o"}}"#,
    ));
    let found = get_agent_by_id(&registry, "db-2").expect("the agent is registered");
    assert!(found.litellm_params.is_empty());
}

#[test]
fn lookup_leaves_the_registry_unchanged() {
    let registry = registry_with_config("alpha");
    let stable = stable_agent(&registry);
    let legacy = legacy_id_of(&registry, &stable.agent_id);
    assert!(get_agent_by_id(&registry, &legacy).is_some());
    assert_eq!(registry.get_agent_list(None).len(), 1);
    assert_eq!(stable_agent(&registry).agent_id, stable.agent_id);
    assert!(registry.ids_for_agent(&stable.agent_id).contains(&legacy));
}
