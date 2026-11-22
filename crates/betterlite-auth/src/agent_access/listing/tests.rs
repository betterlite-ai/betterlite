#![cfg(test)]

use std::collections::BTreeSet;
use std::sync::Mutex;

use rstest::rstest;

use super::{accessible_agents, granted_agent_ids, granted_ids};
use crate::agent_access::tests::{
    ALPHA_LEGACY, ALPHA_STABLE, FakeGrants, FakeHumans, FakeResolver, FakeTargets, alpha_registry,
    auth, calls, ids, managed_row, plain_agent, rejected_status, resolver, restricted,
};
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::{AgentResponse, PolicyFlag};
use crate::resolved_key::auth::UserAPIKeyAuth;

fn listing_resolver(grants: FakeGrants, contexts: &[&str]) -> FakeResolver {
    let humans = FakeHumans {
        reloaded_key: None,
        human_failure: None,
        sources: Vec::new(),
        admit_every_team: false,
        contexts: Some(Ok(contexts
            .iter()
            .map(|user_id| auth(&format!(r#"{{"user_id":"{user_id}"}}"#)))
            .collect())),
        calls: Mutex::new(Vec::new()),
        context_auths: Mutex::new(Vec::new()),
    };
    resolver(grants, FakeTargets::default(), humans)
}

fn agent_ids(agents: &[AgentResponse]) -> Vec<&str> {
    agents.iter().map(|agent| agent.agent_id.as_str()).collect()
}

fn listing_auth(role: &str) -> UserAPIKeyAuth {
    auth(&format!(r#"{{"user_id":"lister","user_role":"{role}"}}"#))
}

#[rstest]
#[case::unrestricted_is_no_ids(AgentAccess::Unrestricted, ids(&[]))]
#[case::restricted_keeps_its_ids(restricted(&["a", "b"]), ids(&["a", "b"]))]
#[case::empty_restricted_is_no_ids(restricted(&[]), ids(&[]))]
fn granted_ids_counts_only_explicit_grants(
    #[case] access: AgentAccess,
    #[case] expected: BTreeSet<String>,
) {
    assert_eq!(granted_ids(access), expected);
}

#[rstest]
#[case::lenient(false)]
#[case::strict(true)]
#[tokio::test]
async fn granted_agent_ids_unions_every_effective_context(#[case] strict: bool) {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("alice"), Ok(restricted(&["a", "b"])));
    grants
        .key_team
        .insert(String::from("bob"), Ok(restricted(&["b", "c"])));
    let access_resolver = listing_resolver(grants, &["alice", "bob", "carol"]);
    let key = auth(r#"{"user_id":"lister"}"#);
    let granted = Box::pin(granted_agent_ids(&access_resolver, &key, strict))
        .await
        .unwrap();
    assert_eq!(granted, ids(&["a", "b", "c"]));
    let key_team_calls: Vec<String> = calls(&access_resolver.grants.calls)
        .into_iter()
        .filter(|call| call.starts_with("key_team"))
        .collect();
    assert_eq!(
        key_team_calls,
        [
            format!("key_team alice strict={strict}"),
            format!("key_team bob strict={strict}"),
            format!("key_team carol strict={strict}"),
        ]
    );
    let asked = access_resolver.humans.context_auths.lock().unwrap();
    assert_eq!(asked[0].view.token.user_id.as_deref(), Some("lister"));
}

#[tokio::test]
async fn granted_agent_ids_returns_the_first_failing_context() {
    let mut grants = FakeGrants::default();
    grants.key_team.insert(String::from("alice"), Err(503));
    grants.key_team.insert(String::from("bob"), Err(401));
    let access_resolver = listing_resolver(grants, &["alice", "bob"]);
    let key = auth(r#"{"user_id":"lister"}"#);
    let outcome = Box::pin(granted_agent_ids(&access_resolver, &key, false)).await;
    assert_eq!(rejected_status(outcome), 503);
}

#[tokio::test]
async fn granted_agent_ids_without_contexts_is_empty() {
    let access_resolver = listing_resolver(FakeGrants::default(), &[]);
    let key = auth(r#"{"user_id":"lister"}"#);
    let granted = Box::pin(granted_agent_ids(&access_resolver, &key, false))
        .await
        .unwrap();
    assert!(granted.is_empty());
}

#[tokio::test]
async fn accessible_agents_returns_every_agent_unchanged_for_a_proxy_admin() {
    let access_resolver = listing_resolver(FakeGrants::default(), &["lister"]);
    let agents = vec![managed_row("m1"), plain_agent("b"), plain_agent("a")];
    let listed = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth("proxy_admin"),
        Some(agents),
    ))
    .await
    .unwrap();
    assert_eq!(agent_ids(&listed), ["m1", "b", "a"]);
    assert!(calls(&access_resolver.grants.calls).is_empty());
    assert!(
        access_resolver
            .humans
            .context_auths
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::internal_user("internal_user")]
#[case::admin_viewer("proxy_admin_viewer")]
#[tokio::test]
async fn accessible_agents_keeps_granted_agents_in_their_order(#[case] role: &str) {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("lister"), Ok(restricted(&["a", "c"])));
    let access_resolver = listing_resolver(grants, &["lister"]);
    let agents = vec![plain_agent("c"), plain_agent("b"), plain_agent("a")];
    let listed = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth(role),
        Some(agents),
    ))
    .await
    .unwrap();
    assert_eq!(agent_ids(&listed), ["c", "a"]);
}

#[tokio::test]
async fn accessible_agents_lists_nothing_for_unrestricted_access() {
    let access_resolver = listing_resolver(FakeGrants::default(), &["lister"]);
    let agents = vec![plain_agent("a")];
    let listed = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth("internal_user"),
        Some(agents),
    ))
    .await
    .unwrap();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn accessible_agents_compares_raw_agent_ids() {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("lister"), Ok(restricted(&[ALPHA_LEGACY])));
    let mut access_resolver = listing_resolver(grants, &["lister"]);
    access_resolver.registry = alpha_registry();
    let agents = vec![plain_agent(ALPHA_LEGACY), plain_agent(ALPHA_STABLE)];
    let listed = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth("internal_user"),
        Some(agents),
    ))
    .await
    .unwrap();
    assert_eq!(agent_ids(&listed), [ALPHA_STABLE]);
}

#[tokio::test]
async fn accessible_agents_keeps_managed_agents_only_when_invocation_is_allowed() {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("lister"), Ok(restricted(&["m1", "m2", "n1"])));
    let mut disabled = managed_row("m2");
    disabled.enabled = PolicyFlag(false);
    let mut targets = FakeTargets::with_database();
    targets
        .agents
        .insert(String::from("m1"), Ok(Some(managed_row("m1"))));
    targets
        .agents
        .insert(String::from("m2"), Ok(Some(disabled.clone())));
    let access_resolver = resolver(grants, targets, FakeHumans::default());
    let agents = vec![
        disabled,
        plain_agent("n1"),
        managed_row("m1"),
        plain_agent("n2"),
    ];
    let listed = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth("internal_user"),
        Some(agents),
    ))
    .await
    .unwrap();
    assert_eq!(agent_ids(&listed), ["n1", "m1"]);
    assert_eq!(
        calls(&access_resolver.targets.calls),
        ["agent m2", "agent m1"]
    );
}

#[tokio::test]
async fn accessible_agents_reads_the_registry_when_no_list_is_given() {
    let mut access_resolver = listing_resolver(FakeGrants::default(), &["lister"]);
    access_resolver.registry = alpha_registry();
    let listed = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth("proxy_admin"),
        None,
    ))
    .await
    .unwrap();
    assert_eq!(agent_ids(&listed), [ALPHA_STABLE]);
}

#[tokio::test]
async fn accessible_agents_returns_a_grant_failure() {
    let mut grants = FakeGrants::default();
    grants.key_team.insert(String::from("lister"), Err(503));
    let access_resolver = listing_resolver(grants, &["lister"]);
    let agents = vec![plain_agent("a")];
    let outcome = Box::pin(accessible_agents(
        &access_resolver,
        &listing_auth("internal_user"),
        Some(agents),
    ))
    .await;
    assert_eq!(rejected_status(outcome), 503);
}
