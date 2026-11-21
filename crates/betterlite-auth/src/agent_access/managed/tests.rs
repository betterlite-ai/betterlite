#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use rstest::rstest;

use super::managed_actor_agent_access;
use crate::agent_access::error::AgentAccessError;
use crate::agent_access::tests::{
    ALPHA_LEGACY, FakeGrants, FakeHumans, FakeResolver, FakeTargets, agent, alpha_registry, auth,
    calls, ids, rejected_status, resolver, restricted,
};
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::ManagedAgentContext;
use crate::resolved_key::auth::UserAPIKeyAuth;

fn managed_key(object_permission: &str, access_group_ids: &str) -> UserAPIKeyAuth {
    let mut key = auth(r#"{"api_key":"sk-agent","user_id":"bot-owner","team_id":"team-1"}"#);
    key.extra.managed_agent_policy = Some(agent(&format!(
        r#"{{"agent_id":"bot","agent_name":"bot","agent_card_params":{{}},"identity_managed":true,"object_permission":{object_permission},"access_group_ids":{access_group_ids}}}"#
    )));
    key
}

fn declared_key() -> UserAPIKeyAuth {
    managed_key(
        r#"{"object_permission_id":"op","agents":["a","b","c"]}"#,
        "null",
    )
}

fn owning(grants: FakeGrants) -> FakeResolver {
    resolver(grants, FakeTargets::default(), FakeHumans::default())
}

fn own_grants(agent_ids: &[&str]) -> FakeGrants {
    FakeGrants {
        key_team: BTreeMap::new(),
        caller: Ok(AgentAccess::Unrestricted),
        key_strict: Ok(restricted(agent_ids)),
        ceiling: Ok(None),
        managed_ceilings: Ok(Vec::new()),
        calls: Mutex::new(Vec::new()),
    }
}

fn context(text: &str) -> ManagedAgentContext {
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn a_key_without_a_managed_policy_reaches_nothing() {
    let access_resolver = owning(own_grants(&["a"]));
    let key = auth(r#"{"user_id":"alice"}"#);
    let access = Box::pin(managed_actor_agent_access(&access_resolver, &key))
        .await
        .unwrap();
    assert_eq!(access, restricted(&[]));
    assert!(calls(&access_resolver.grants.calls).is_empty());
}

#[rstest]
#[case::absent("null")]
#[case::empty_map("{}")]
#[tokio::test]
async fn a_missing_or_empty_object_permission_reaches_nothing(#[case] object_permission: &str) {
    let access_resolver = owning(own_grants(&["a"]));
    let key = managed_key(object_permission, "null");
    let access = Box::pin(managed_actor_agent_access(&access_resolver, &key))
        .await
        .unwrap();
    assert_eq!(access, restricted(&[]));
    assert!(calls(&access_resolver.grants.calls).is_empty());
}

#[tokio::test]
async fn an_object_permission_that_does_not_parse_is_an_error() {
    let access_resolver = owning(own_grants(&["a"]));
    let key = managed_key(r#"{"agents":["a"]}"#, "null");
    let outcome = Box::pin(managed_actor_agent_access(&access_resolver, &key)).await;
    assert!(matches!(
        &outcome,
        Err(AgentAccessError::ObjectPermission { agent_id, .. }) if agent_id == "bot"
    ));
    assert_eq!(rejected_status(outcome), 500);
}

#[tokio::test]
async fn own_grants_come_from_the_declared_object_permission() {
    let access_resolver = owning(own_grants(&["a", "b"]));
    let key = managed_key(
        r#"{"object_permission_id":"op","agents":["a","b","c"]}"#,
        r#"["g1","g2"]"#,
    );
    let access = Box::pin(managed_actor_agent_access(&access_resolver, &key))
        .await
        .unwrap();
    assert_eq!(access, restricted(&["a", "b"]));
    assert_eq!(
        calls(&access_resolver.grants.calls),
        [
            "key_strict anonymous agents=Some([\"a\", \"b\", \"c\"])",
            "managed_ceilings bot groups=[\"g1\", \"g2\"]",
            "caller team-1 strict=true",
        ]
    );
}

#[tokio::test]
async fn an_unrestricted_own_grant_reaches_nothing() {
    let access_resolver = owning(FakeGrants::default());
    let access = Box::pin(managed_actor_agent_access(
        &access_resolver,
        &declared_key(),
    ))
    .await
    .unwrap();
    assert_eq!(access, restricted(&[]));
}

#[rstest]
#[case::no_ceiling(vec![], &["a", "b", "c"])]
#[case::one_ceiling(vec![ids(&["a", "b", "z"])], &["a", "b"])]
#[case::every_ceiling_must_allow(vec![ids(&["a", "b"]), ids(&["b", "c"])], &["b"])]
#[case::empty_ceiling_denies(vec![ids(&[])], &[])]
#[tokio::test]
async fn every_ceiling_narrows_the_own_grants(
    #[case] ceilings: Vec<BTreeSet<String>>,
    #[case] expected: &[&str],
) {
    let mut grants = own_grants(&["a", "b", "c"]);
    grants.managed_ceilings = Ok(ceilings);
    let access_resolver = owning(grants);
    let access = Box::pin(managed_actor_agent_access(
        &access_resolver,
        &declared_key(),
    ))
    .await
    .unwrap();
    assert_eq!(access, restricted(expected));
}

#[rstest]
#[case::unrestricted_caller(AgentAccess::Unrestricted, &["a", "b", "c"])]
#[case::restricted_caller(restricted(&["b", "z"]), &["b"])]
#[case::caller_with_no_grant(restricted(&[]), &[])]
#[tokio::test]
async fn the_caller_grants_cap_the_grouped_ids(
    #[case] caller: AgentAccess,
    #[case] expected: &[&str],
) {
    let mut grants = own_grants(&["a", "b", "c"]);
    grants.caller = Ok(caller);
    let access_resolver = owning(grants);
    let access = Box::pin(managed_actor_agent_access(
        &access_resolver,
        &declared_key(),
    ))
    .await
    .unwrap();
    assert_eq!(access, restricted(expected));
}

#[tokio::test]
async fn managed_ids_are_never_mapped_to_stable_ids() {
    let mut grants = own_grants(&[ALPHA_LEGACY]);
    grants.caller = Ok(restricted(&[ALPHA_LEGACY]));
    grants.managed_ceilings = Ok(vec![ids(&[ALPHA_LEGACY])]);
    let mut access_resolver = owning(grants);
    access_resolver.registry = alpha_registry();
    let access = Box::pin(managed_actor_agent_access(
        &access_resolver,
        &declared_key(),
    ))
    .await
    .unwrap();
    assert_eq!(access, restricted(&[ALPHA_LEGACY]));
}

#[rstest]
#[case::autonomous(r#"{"agent_id":"bot","mode":"autonomous","user_id":"human"}"#, &["a", "b", "c"])]
#[case::delegated_without_a_user(r#"{"agent_id":"bot","mode":"delegated"}"#, &[])]
#[tokio::test]
async fn the_context_mode_decides_without_a_human(#[case] given: &str, #[case] expected: &[&str]) {
    let access_resolver = owning(own_grants(&["a", "b", "c"]));
    let mut key = declared_key();
    key.extra.managed_agent_context = Some(context(given));
    let access = Box::pin(managed_actor_agent_access(&access_resolver, &key))
        .await
        .unwrap();
    assert_eq!(access, restricted(expected));
    assert!(calls(&access_resolver.humans.calls).is_empty());
}

#[tokio::test]
async fn a_delegated_context_keeps_what_the_human_may_reach_through_the_key_team() {
    let mut grants = own_grants(&["a", "b", "c"]);
    grants
        .key_team
        .insert(String::from("human"), Ok(restricted(&["a", "x"])));
    grants
        .key_team
        .insert(String::from("team-1"), Ok(restricted(&["b"])));
    grants
        .key_team
        .insert(String::from("team-2"), Ok(restricted(&["c"])));
    let humans = FakeHumans::admitting(vec![
        auth(r#"{"user_id":"human"}"#),
        auth(r#"{"user_id":"human","team_id":"team-1"}"#),
        auth(r#"{"user_id":"human","team_id":"team-2"}"#),
    ]);
    let access_resolver = resolver(grants, FakeTargets::default(), humans);
    let mut key = declared_key();
    key.extra.managed_agent_context = Some(context(
        r#"{"agent_id":"bot","mode":"delegated","user_id":"human"}"#,
    ));
    let access = Box::pin(managed_actor_agent_access(&access_resolver, &key))
        .await
        .unwrap();
    assert_eq!(access, restricted(&["a", "b"]));
    assert_eq!(
        calls(&access_resolver.humans.calls),
        [
            "reload_user human fresh=true",
            "sources human allowed=Some({\"team-1\"})",
        ]
    );
}

#[derive(Clone, Copy)]
enum FailingPort {
    OwnGrants,
    Ceilings,
    Caller,
    HumanReload,
}

#[rstest]
#[case::own_grants(FailingPort::OwnGrants)]
#[case::ceilings(FailingPort::Ceilings)]
#[case::caller(FailingPort::Caller)]
#[case::human_reload(FailingPort::HumanReload)]
#[tokio::test]
async fn a_failing_port_fails_the_managed_access(#[case] failing: FailingPort) {
    let mut grants = own_grants(&["a"]);
    let mut humans = FakeHumans::default();
    match failing {
        FailingPort::OwnGrants => grants.key_strict = Err(503),
        FailingPort::Ceilings => grants.managed_ceilings = Err(503),
        FailingPort::Caller => grants.caller = Err(503),
        FailingPort::HumanReload => humans.human_failure = Some(503),
    }
    let access_resolver = resolver(grants, FakeTargets::default(), humans);
    let mut key = declared_key();
    key.extra.managed_agent_context = Some(context(
        r#"{"agent_id":"bot","mode":"delegated","user_id":"human"}"#,
    ));
    let outcome = Box::pin(managed_actor_agent_access(&access_resolver, &key)).await;
    assert_eq!(rejected_status(outcome), 503);
}
