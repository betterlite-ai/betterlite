#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::resolve_agent_access;
use crate::agent_access::tests::{
    ALPHA_LEGACY, ALPHA_STABLE, FakeGrants, FakeHumans, FakeTargets, agent, alpha_registry, auth,
    calls, ids, resolver, restricted,
};
use crate::agent_access_groups::access::AgentAccess;

fn grants(
    key_team: AgentAccess,
    caller: AgentAccess,
    ceiling: Option<BTreeSet<String>>,
) -> FakeGrants {
    let mut grants = FakeGrants::default();
    grants.key_team.insert(String::from("alice"), Ok(key_team));
    grants.caller = Ok(caller);
    grants.ceiling = Ok(ceiling);
    grants
}

#[rstest]
#[case::everything_unrestricted(
    AgentAccess::Unrestricted,
    AgentAccess::Unrestricted,
    None,
    AgentAccess::Unrestricted
)]
#[case::no_ceiling_keeps_own_access(restricted(&["a", "b"]), AgentAccess::Unrestricted, None, restricted(&["a", "b"]))]
#[case::caller_narrows_key_and_team(restricted(&["a", "b"]), restricted(&["b", "c"]), None, restricted(&["b"]))]
#[case::caller_alone_restricts(AgentAccess::Unrestricted, restricted(&["c"]), None, restricted(&["c"]))]
#[case::unrestricted_own_becomes_the_ceiling(AgentAccess::Unrestricted, AgentAccess::Unrestricted, Some(ids(&["c", "d"])), restricted(&["c", "d"]))]
#[case::restricted_own_meets_the_ceiling(restricted(&["a", "b"]), AgentAccess::Unrestricted, Some(ids(&["b", "c"])), restricted(&["b"]))]
#[case::empty_ceiling_denies(restricted(&["a"]), AgentAccess::Unrestricted, Some(ids(&[])), restricted(&[]))]
#[tokio::test]
async fn own_access_is_capped_by_the_caller_and_the_ceiling(
    #[case] key_team: AgentAccess,
    #[case] caller: AgentAccess,
    #[case] ceiling: Option<BTreeSet<String>>,
    #[case] expected: AgentAccess,
) {
    let access_resolver = resolver(
        grants(key_team, caller, ceiling),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    let key = auth(r#"{"user_id":"alice"}"#);
    let access = resolve_agent_access(&access_resolver, Some(&key), false)
        .await
        .unwrap();
    assert_eq!(access, expected);
}

#[tokio::test]
async fn every_port_is_read_in_order_with_the_same_auth_and_mode() {
    let access_resolver = resolver(
        FakeGrants::default(),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    let access = resolve_agent_access(&access_resolver, None, false)
        .await
        .unwrap();
    assert_eq!(access, AgentAccess::Unrestricted);
    assert_eq!(
        calls(&access_resolver.grants.calls),
        [
            "key_team none strict=false",
            "caller none strict=false",
            "ceiling none strict=false",
        ]
    );
}

#[tokio::test]
async fn own_access_is_mapped_to_stable_ids_before_the_ceiling() {
    let mut access_resolver = resolver(
        grants(
            restricted(&[ALPHA_LEGACY, "db-agent"]),
            AgentAccess::Unrestricted,
            Some(ids(&[ALPHA_STABLE])),
        ),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    access_resolver.registry = alpha_registry();
    let key = auth(r#"{"user_id":"alice"}"#);
    let access = resolve_agent_access(&access_resolver, Some(&key), false)
        .await
        .unwrap();
    assert_eq!(access, restricted(&[ALPHA_STABLE]));
}

#[tokio::test]
async fn a_managed_key_returns_its_managed_access_without_key_or_team_grants() {
    let mut grants = grants(
        restricted(&["key-only"]),
        AgentAccess::Unrestricted,
        Some(ids(&["ceiling-only"])),
    );
    grants.key_strict = Ok(restricted(&["a", "b"]));
    let access_resolver = resolver(grants, FakeTargets::default(), FakeHumans::default());
    let mut key = auth(r#"{"user_id":"alice"}"#);
    key.extra.managed_agent_policy = Some(agent(
        r#"{"agent_id":"bot","agent_name":"bot","agent_card_params":{},"identity_managed":true,"object_permission":{"object_permission_id":"op","agents":["a","b"]}}"#,
    ));
    let access = resolve_agent_access(&access_resolver, Some(&key), false)
        .await
        .unwrap();
    assert_eq!(access, restricted(&["a", "b"]));
    assert_eq!(
        calls(&access_resolver.grants.calls),
        [
            "key_strict anonymous agents=Some([\"a\", \"b\"])",
            "managed_ceilings bot groups=[]",
            "caller alice strict=true",
        ]
    );
}
