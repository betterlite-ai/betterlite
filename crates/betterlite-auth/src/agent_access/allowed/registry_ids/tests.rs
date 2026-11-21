#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use crate::agent_access::allowed::is_agent_allowed;
use crate::agent_access::tests::{
    ALPHA_LEGACY, ALPHA_STABLE, FakeGrants, FakeHumans, FakeTargets, alpha_registry, auth, calls,
    ids, plain_agent, resolver, restricted,
};
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistryApi as _;

#[tokio::test]
async fn registry_unrestricted_access_allows_every_agent() {
    let access_resolver = resolver(
        FakeGrants::default(),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    let key = auth(r#"{"user_id":"alice"}"#);
    let allowed = is_agent_allowed(&access_resolver, "any-agent", Some(&key))
        .await
        .unwrap();
    assert!(allowed);
    assert!(calls(&access_resolver.targets.calls).is_empty());
    assert_eq!(
        calls(&access_resolver.grants.calls),
        [
            "key_team alice strict=false",
            "caller alice strict=false",
            "ceiling alice strict=false",
        ]
    );
}

#[rstest]
#[case::legacy_grant_reaches_the_legacy_id(restricted(&[ALPHA_LEGACY]), None, ALPHA_LEGACY, true)]
#[case::legacy_grant_reaches_the_stable_id(restricted(&[ALPHA_LEGACY]), None, ALPHA_STABLE, true)]
#[case::stable_grant_reaches_the_legacy_id(restricted(&[ALPHA_STABLE]), None, ALPHA_LEGACY, true)]
#[case::legacy_ceiling_reaches_the_stable_id(AgentAccess::Unrestricted, Some(ids(&[ALPHA_LEGACY])), ALPHA_STABLE, true)]
#[case::db_agent_grant_matches_its_raw_id(restricted(&["db-agent"]), None, "db-agent", true)]
#[case::other_agent_is_not_granted(restricted(&[ALPHA_STABLE]), None, "db-agent", false)]
#[case::empty_grant_denies(restricted(&[]), None, ALPHA_STABLE, false)]
#[tokio::test]
async fn registry_restricted_access_matches_stable_and_legacy_ids(
    #[case] key_team: AgentAccess,
    #[case] ceiling: Option<BTreeSet<String>>,
    #[case] agent_id: &str,
    #[case] expected: bool,
) {
    let mut grants = FakeGrants::default();
    grants.key_team.insert(String::from("alice"), Ok(key_team));
    grants.ceiling = Ok(ceiling);
    let mut access_resolver = resolver(grants, FakeTargets::default(), FakeHumans::default());
    access_resolver.registry = alpha_registry();
    let key = auth(r#"{"user_id":"alice"}"#);
    let allowed = is_agent_allowed(&access_resolver, agent_id, Some(&key))
        .await
        .unwrap();
    assert_eq!(allowed, expected);
}

#[rstest]
#[case::row_that_is_not_identity_managed(Some(plain_agent("db-agent")))]
#[case::missing_row_of_an_unmanaged_registry_agent(None)]
#[tokio::test]
async fn registry_store_rows_that_are_not_managed_fall_through_to_the_grants(
    #[case] row: Option<AgentResponse>,
) {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("alice"), Ok(restricted(&["db-agent"])));
    let mut targets = FakeTargets::default();
    targets.database = true;
    targets.agents.insert(String::from("db-agent"), Ok(row));
    let access_resolver = resolver(grants, targets, FakeHumans::default());
    access_resolver
        .registry
        .register_agent(plain_agent("db-agent"));
    let key = auth(r#"{"user_id":"alice"}"#);
    let granted = is_agent_allowed(&access_resolver, "db-agent", Some(&key))
        .await
        .unwrap();
    let denied = is_agent_allowed(&access_resolver, "other", Some(&key))
        .await
        .unwrap();
    assert!(granted);
    assert!(!denied);
    assert_eq!(
        calls(&access_resolver.targets.calls),
        ["agent db-agent", "agent other"]
    );
}

#[tokio::test]
async fn registry_without_a_database_or_managed_copy_skips_the_store() {
    let access_resolver = resolver(
        FakeGrants::default(),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    access_resolver
        .registry
        .register_agent(plain_agent("db-agent"));
    let allowed = is_agent_allowed(&access_resolver, "db-agent", None)
        .await
        .unwrap();
    assert!(allowed);
    assert!(calls(&access_resolver.targets.calls).is_empty());
}
