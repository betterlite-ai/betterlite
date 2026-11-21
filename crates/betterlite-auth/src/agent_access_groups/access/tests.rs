#![cfg(test)]
use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::{AgentAccess, intersect_agent_access, to_stable_ids};
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;

const ALPHA_STABLE: &str = "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8";
const ALPHA_LEGACY: &str = "00afdb1e315efd7d8d96382f3925c7d46dee40711cffe80449b0dcb21f731322";

fn registry() -> AgentRegistry {
    let alpha: Json =
        serde_json::from_str(r#"{"agent_name":"alpha","agent_card_params":{"model":"gpt-4o"}}"#)
            .unwrap();
    let registry = AgentRegistry::default();
    registry
        .load_agents_from_config(Some([alpha].as_slice()))
        .unwrap();
    registry
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn restricted(values: &[&str]) -> AgentAccess {
    AgentAccess::Restricted(ids(values))
}

#[test]
fn the_fixture_registry_maps_the_legacy_alpha_id() {
    assert_eq!(registry().stable_agent_id(ALPHA_LEGACY), ALPHA_STABLE);
}

#[rstest]
#[case::legacy_id(&[ALPHA_LEGACY, "db-agent"], &[ALPHA_STABLE, "db-agent"])]
#[case::stable_id(&[ALPHA_STABLE], &[ALPHA_STABLE])]
#[case::legacy_and_stable_fold_into_one(&[ALPHA_LEGACY, ALPHA_STABLE], &[ALPHA_STABLE])]
#[case::unknown_ids_stay(&["missing", "other"], &["missing", "other"])]
#[case::empty(&[], &[])]
fn to_stable_ids_maps_legacy_config_ids_and_keeps_the_rest(
    #[case] given: &[&str],
    #[case] expected: &[&str],
) {
    assert_eq!(to_stable_ids(&registry(), &ids(given)), ids(expected));
}

#[test]
fn both_sides_unrestricted_stay_unrestricted() {
    let access = intersect_agent_access(
        &registry(),
        &AgentAccess::Unrestricted,
        &AgentAccess::Unrestricted,
    );
    assert_eq!(access, AgentAccess::Unrestricted);
}

#[rstest]
#[case::key_restricted(restricted(&[ALPHA_LEGACY, "db-agent"]), AgentAccess::Unrestricted)]
#[case::team_restricted(AgentAccess::Unrestricted, restricted(&[ALPHA_LEGACY, "db-agent"]))]
fn one_restricted_side_gives_its_stable_ids(
    #[case] key_access: AgentAccess,
    #[case] team_access: AgentAccess,
) {
    let access = intersect_agent_access(&registry(), &key_access, &team_access);
    assert_eq!(access, restricted(&[ALPHA_STABLE, "db-agent"]));
}

#[rstest]
#[case::key_restricted(restricted(&[]), AgentAccess::Unrestricted)]
#[case::team_restricted(AgentAccess::Unrestricted, restricted(&[]))]
fn an_empty_restricted_side_denies_every_agent(
    #[case] key_access: AgentAccess,
    #[case] team_access: AgentAccess,
) {
    let access = intersect_agent_access(&registry(), &key_access, &team_access);
    assert_eq!(access, restricted(&[]));
}

#[test]
fn both_restricted_sides_intersect_on_stable_ids() {
    let access = intersect_agent_access(
        &registry(),
        &restricted(&[ALPHA_LEGACY, "db-agent", "key-only"]),
        &restricted(&[ALPHA_STABLE, "db-agent", "team-only"]),
    );
    assert_eq!(access, restricted(&[ALPHA_STABLE, "db-agent"]));
}

#[test]
fn disjoint_restricted_sides_deny_every_agent() {
    let access = intersect_agent_access(
        &registry(),
        &restricted(&["key-only"]),
        &restricted(&["team-only"]),
    );
    assert_eq!(access, restricted(&[]));
}
