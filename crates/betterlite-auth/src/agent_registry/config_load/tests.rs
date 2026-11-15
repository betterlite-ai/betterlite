#![cfg(test)]

use std::collections::BTreeSet;

use super::AGENT_FIELDS;
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::AgentRegistryError;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_LEGACY, ALPHA_STABLE, BETA_ENTRY, BETA_STABLE, agent, entries, loaded,
};

fn names(registry: &AgentRegistry) -> Vec<String> {
    let agents = registry.get_agent_list(None);
    agents.into_iter().map(|agent| agent.agent_name).collect()
}

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn none_leaves_the_registry_untouched() {
    let registry = loaded(&[ALPHA_ENTRY]);
    registry.load_agents_from_config(None).unwrap();
    assert_eq!(names(&registry), vec![String::from("alpha")]);
    assert_eq!(
        registry.ids_for_agent(ALPHA_STABLE),
        set(&[ALPHA_STABLE, ALPHA_LEGACY])
    );
}

#[test]
fn a_non_object_entry_is_rejected_before_any_change() {
    let registry = loaded(&[ALPHA_ENTRY]);
    let config = entries(&[BETA_ENTRY, "5"]);
    let error = registry.load_agents_from_config(Some(&config)).unwrap_err();
    assert!(matches!(error, AgentRegistryError::NotAListOfDictionaries));
    assert_eq!(
        error.to_string(),
        "agent_config must be a list of dictionaries"
    );
    assert_eq!(names(&registry), vec![String::from("alpha")]);
    assert_eq!(registry.stable_agent_id(BETA_STABLE), BETA_STABLE);
    let rebuilt = registry.load_agents_from_db_and_config(None, None).unwrap();
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].agent_name, "alpha");
}

#[test]
fn entries_register_in_order_with_their_stable_ids() {
    let registry = AgentRegistry::default();
    let config = entries(&[ALPHA_ENTRY, BETA_ENTRY]);
    registry.load_agents_from_config(Some(&config)).unwrap();
    assert_eq!(
        names(&registry),
        vec![String::from("alpha"), String::from("b\u{e9}ta")]
    );
    let agents = registry.get_agent_list(None);
    assert_eq!(agents[0].agent_id, ALPHA_STABLE);
    assert_eq!(agents[1].agent_id, BETA_STABLE);
}

#[test]
fn incomplete_entries_are_skipped_without_error() {
    let registry = AgentRegistry::default();
    let config = entries(&[
        r#"{"agent_name":"","agent_card_params":{"x":1}}"#,
        r#"{"agent_name":"nocard"}"#,
        r#"{"agent_name":"nullcard","agent_card_params":null}"#,
        r#"{"agent_name":"emptycard","agent_card_params":{}}"#,
        BETA_ENTRY,
    ]);
    registry.load_agents_from_config(Some(&config)).unwrap();
    assert_eq!(names(&registry), vec![String::from("b\u{e9}ta")]);
    assert_eq!(registry.ids_for_agent(BETA_STABLE).len(), 2);
}

#[test]
fn a_name_already_registered_wins_and_still_enters_the_legacy_map() {
    let registry = AgentRegistry::default();
    registry.register_agent(agent("db-alpha", "alpha"));
    let config = entries(&[ALPHA_ENTRY, BETA_ENTRY]);
    registry.load_agents_from_config(Some(&config)).unwrap();
    assert_eq!(
        names(&registry),
        vec![String::from("alpha"), String::from("b\u{e9}ta")]
    );
    assert_eq!(registry.get_agent_list(None)[0].agent_id, "db-alpha");
    assert_eq!(
        registry.ids_for_agent(ALPHA_STABLE),
        set(&[ALPHA_STABLE, ALPHA_LEGACY])
    );
}

#[test]
fn two_entries_with_one_name_register_only_the_first() {
    let second = r#"{"agent_name":"alpha","agent_card_params":{"model":"claude-sonnet-5-5"}}"#;
    let registry = loaded(&[ALPHA_ENTRY, second]);
    assert_eq!(names(&registry), vec![String::from("alpha")]);
    let agents = registry.get_agent_list(None);
    let model = agents[0]
        .agent_card_params
        .get("model")
        .map(|card| card.get());
    assert_eq!(model, Some("\"gpt-4o\""));
    assert_eq!(registry.ids_for_agent(ALPHA_STABLE).len(), 3);
}

#[test]
fn an_empty_slice_clears_the_remembered_config_but_not_the_list() {
    let registry = loaded(&[ALPHA_ENTRY]);
    registry.load_agents_from_config(Some(&[])).unwrap();
    assert_eq!(names(&registry), vec![String::from("alpha")]);
    assert_eq!(registry.ids_for_agent(ALPHA_STABLE), set(&[ALPHA_STABLE]));
    let rebuilt = registry.load_agents_from_db_and_config(None, None).unwrap();
    assert!(rebuilt.is_empty());
}

#[test]
fn unknown_keys_are_rejected_by_name() {
    let registry = AgentRegistry::default();
    let config = entries(&[r#"{"agent_name":"gamma","agent_card_params":{"x":1},"team_id":"t"}"#]);
    let error = registry.load_agents_from_config(Some(&config)).unwrap_err();
    let AgentRegistryError::UnknownConfigKey { key } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(key, "team_id");
}

#[test]
fn an_entry_cannot_set_agent_id() {
    let registry = AgentRegistry::default();
    let config =
        entries(&[r#"{"agent_name":"gamma","agent_card_params":{"x":1},"agent_id":"given"}"#]);
    let error = registry.load_agents_from_config(Some(&config)).unwrap_err();
    assert!(matches!(error, AgentRegistryError::ConfigAgentId));
}

#[test]
fn wrongly_typed_name_and_card_fields_are_named() {
    let registry = AgentRegistry::default();
    let bad_name = entries(&[r#"{"agent_name":5,"agent_card_params":{"x":1}}"#]);
    let name_error = registry
        .load_agents_from_config(Some(&bad_name))
        .unwrap_err();
    assert!(name_error.to_string().contains("agent_name"));
    let bad_card = entries(&[r#"{"agent_name":"gamma","agent_card_params":"x"}"#]);
    let card_error = registry
        .load_agents_from_config(Some(&bad_card))
        .unwrap_err();
    assert!(card_error.to_string().contains("agent_card_params"));
}

#[test]
fn a_wrongly_typed_field_is_reported_with_its_path() {
    let registry = AgentRegistry::default();
    let config =
        entries(&[r#"{"agent_name":"gamma","agent_card_params":{"x":1},"tpm_limit":"many"}"#]);
    let error = registry.load_agents_from_config(Some(&config)).unwrap_err();
    let AgentRegistryError::Record { path, .. } = &error else {
        panic!("unexpected error: {error}");
    };
    assert_eq!(path, "tpm_limit");
}

#[test]
fn the_known_field_list_matches_the_record() {
    let value = serde_json::to_value(agent("id", "name")).unwrap();
    let object = value.as_object().unwrap();
    let mut found: Vec<&str> = object.keys().map(String::as_str).collect();
    found.sort_unstable();
    let mut expected = AGENT_FIELDS.to_vec();
    expected.sort_unstable();
    assert_eq!(found, expected);
}
