#![cfg(test)]

use std::collections::BTreeSet;

use super::stable_id;
use crate::agent_registry::AgentRegistryApi as _;
use crate::agent_registry::tests::support::{
    ALPHA_ENTRY, ALPHA_LEGACY, ALPHA_STABLE, BETA_ENTRY, BETA_LEGACY, BETA_STABLE, FLOATS_ENTRY,
    FLOATS_LEGACY, FLOATS_STABLE, loaded,
};

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[test]
fn stable_ids_are_the_lowercase_sha256_of_the_name() {
    assert_eq!(stable_id("alpha"), ALPHA_STABLE);
    assert_eq!(stable_id("béta"), BETA_STABLE);
    assert_eq!(stable_id("floats"), FLOATS_STABLE);
}

#[test]
fn ascii_legacy_ids_match_pythons_sorted_json_dumps() {
    let registry = loaded(&[ALPHA_ENTRY]);
    assert_eq!(
        registry.ids_for_agent(ALPHA_STABLE),
        set(&[ALPHA_STABLE, ALPHA_LEGACY])
    );
}

#[test]
fn escaped_legacy_ids_match_pythons_sorted_json_dumps() {
    let registry = loaded(&[BETA_ENTRY]);
    assert_eq!(
        registry.ids_for_agent(BETA_STABLE),
        set(&[BETA_STABLE, BETA_LEGACY])
    );
}

#[test]
fn float_legacy_ids_match_pythons_sorted_json_dumps() {
    let registry = loaded(&[FLOATS_ENTRY]);
    assert_eq!(
        registry.ids_for_agent(FLOATS_STABLE),
        set(&[FLOATS_STABLE, FLOATS_LEGACY])
    );
}

#[test]
fn ids_for_agent_collects_every_legacy_alias_of_one_name() {
    let first = r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#;
    let second = r#"{"agent_name":"alpha","agent_card_params":{"model":"claude-sonnet-5-5"}}"#;
    let registry = loaded(&[first, second]);
    let ids = registry.ids_for_agent(ALPHA_STABLE);
    assert_eq!(ids.len(), 3);
    let aliases: BTreeSet<String> = ids
        .iter()
        .filter(|id| id.as_str() != ALPHA_STABLE)
        .cloned()
        .collect();
    assert_eq!(aliases.len(), 2);
    assert!(
        aliases
            .iter()
            .all(|id| registry.stable_agent_id(id) == ALPHA_STABLE)
    );
}

#[test]
fn stable_agent_id_maps_known_legacy_ids_and_leaves_others_alone() {
    let registry = loaded(&[ALPHA_ENTRY]);
    assert_eq!(registry.stable_agent_id(ALPHA_LEGACY), ALPHA_STABLE);
    assert_eq!(registry.stable_agent_id(ALPHA_STABLE), ALPHA_STABLE);
    assert_eq!(registry.stable_agent_id("unknown"), "unknown");
}

#[test]
fn ids_for_agent_of_an_unknown_id_is_that_id_alone() {
    let registry = loaded(&[ALPHA_ENTRY]);
    assert_eq!(registry.ids_for_agent("unknown"), set(&["unknown"]));
}
