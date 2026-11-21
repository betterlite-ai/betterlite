#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    DelegatedTeamRequest, HumanGrant, resolve_delegated_agent_team, verified_human_agent_grants,
    verified_human_sources,
};
use crate::agent_access::error::AgentAccessError;
use crate::agent_access::tests::{
    FakeGrants, FakeHumans, FakeResolver, FakeTargets, auth, calls, ids, rejected_status, resolver,
    restricted,
};

// the user's own source grants "own", team-1 grants "a" and "shared", team-2 grants "b" and "shared"
fn human_resolver() -> FakeResolver {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(String::from("human"), Ok(restricted(&["own"])));
    grants
        .key_team
        .insert(String::from("team-1"), Ok(restricted(&["a", "shared"])));
    grants
        .key_team
        .insert(String::from("team-2"), Ok(restricted(&["b", "shared"])));
    let mut humans = FakeHumans::default();
    humans.sources = vec![
        auth(r#"{"user_id":"human"}"#),
        auth(r#"{"user_id":"human","team_id":"team-2"}"#),
        auth(r#"{"user_id":"human","team_id":"team-1"}"#),
        auth(r#"{"user_id":"human","team_id":"team-3"}"#),
    ];
    resolver(grants, FakeTargets::default(), humans)
}

fn grant(team_id: Option<&str>, agent_ids: &[&str]) -> HumanGrant {
    HumanGrant {
        team_id: team_id.map(str::to_owned),
        agent_ids: ids(agent_ids),
    }
}

#[tokio::test]
async fn verified_human_sources_without_a_user_is_empty() {
    let access_resolver = human_resolver();
    let sources = verified_human_sources(&access_resolver, None, None)
        .await
        .unwrap();
    assert!(sources.is_empty());
    assert!(calls(&access_resolver.humans.calls).is_empty());
}

#[tokio::test]
async fn verified_human_sources_pair_each_source_with_its_strict_grants_in_order() {
    let access_resolver = human_resolver();
    let sources = verified_human_sources(&access_resolver, Some("human"), None)
        .await
        .unwrap();
    assert_eq!(
        sources,
        [
            grant(None, &["own"]),
            grant(Some("team-2"), &["b", "shared"]),
            grant(Some("team-1"), &["a", "shared"]),
            grant(Some("team-3"), &[]),
        ]
    );
    assert_eq!(
        calls(&access_resolver.humans.calls),
        ["reload_user human fresh=true", "sources human allowed=None"]
    );
    let key_team_calls: Vec<String> = calls(&access_resolver.grants.calls)
        .into_iter()
        .filter(|call| call.starts_with("key_team"))
        .collect();
    assert_eq!(
        key_team_calls,
        [
            "key_team human strict=true",
            "key_team team-2 strict=true",
            "key_team team-1 strict=true",
            "key_team team-3 strict=true",
        ]
    );
}

#[tokio::test]
async fn verified_human_sources_return_a_reload_failure() {
    let mut access_resolver = human_resolver();
    access_resolver.humans.human_failure = Some(401);
    let outcome = verified_human_sources(&access_resolver, Some("human"), None).await;
    assert_eq!(rejected_status(outcome), 401);
}

#[rstest]
#[case::team_given(Some("team-1"), "Some({\"team-1\"})", &["a", "own", "shared"])]
#[case::no_team_admits_no_team_source(None, "Some({})", &["own"])]
#[case::team_the_human_is_not_in(Some("team-9"), "Some({\"team-9\"})", &["own"])]
#[tokio::test]
async fn verified_human_agent_grants_ask_for_the_allowed_teams_and_union_own_and_team(
    #[case] team_id: Option<&str>,
    #[case] allowed: &str,
    #[case] expected: &[&str],
) {
    let access_resolver = human_resolver();
    let granted = verified_human_agent_grants(&access_resolver, Some("human"), team_id)
        .await
        .unwrap();
    assert_eq!(granted, ids(expected));
    assert_eq!(
        calls(&access_resolver.humans.calls)[1],
        format!("sources human allowed={allowed}")
    );
}

#[tokio::test]
async fn verified_human_agent_grants_keep_only_the_own_and_given_team_sources() {
    let mut access_resolver = human_resolver();
    access_resolver.humans.admit_every_team = true;
    let with_team = verified_human_agent_grants(&access_resolver, Some("human"), Some("team-1"))
        .await
        .unwrap();
    let without_team = verified_human_agent_grants(&access_resolver, Some("human"), None)
        .await
        .unwrap();
    assert_eq!(with_team, ids(&["a", "own", "shared"]));
    assert_eq!(without_team, ids(&["own"]));
}

#[tokio::test]
async fn verified_human_agent_grants_without_a_user_are_empty() {
    let access_resolver = human_resolver();
    let granted = verified_human_agent_grants(&access_resolver, None, Some("team-1"))
        .await
        .unwrap();
    assert!(granted.is_empty());
}

#[rstest]
#[case::user_grant_keeps_the_given_team("own", Some("team-9"), true, None, Ok(Some("team-9")))]
#[case::user_grant_without_a_team("own", None, false, None, Ok(None))]
#[case::user_grant_ignores_the_allowed_teams("own", Some("team-9"), true, Some(&["team-1"][..]), Ok(Some("team-9")))]
#[case::chosen_team_grants("shared", Some("team-2"), true, None, Ok(Some("team-2")))]
#[case::unchosen_team_takes_the_smallest_granting_team(
    "shared",
    None,
    false,
    None,
    Ok(Some("team-1"))
)]
#[case::implicit_team_that_does_not_grant_takes_the_smallest(
    "b",
    Some("team-1"),
    false,
    None,
    Ok(Some("team-2"))
)]
#[case::explicit_team_that_does_not_grant_is_rejected("b", Some("team-1"), true, None, Err(403))]
#[case::no_granting_team_is_rejected("missing", None, false, None, Err(403))]
#[case::allowed_teams_filter_the_granting_teams("shared", None, false, Some(&["team-2"][..]), Ok(Some("team-2")))]
#[case::allowed_teams_reject_a_chosen_team("shared", Some("team-1"), true, Some(&["team-2"][..]), Err(403))]
#[case::empty_allowed_teams_reject_every_team("shared", None, false, Some(&[][..]), Err(403))]
#[tokio::test]
async fn resolve_delegated_agent_team_picks_the_granting_team(
    #[case] agent_id: &str,
    #[case] team_id: Option<&str>,
    #[case] explicit_team: bool,
    #[case] allowed: Option<&[&str]>,
    #[case] expected: Result<Option<&str>, u16>,
) {
    let access_resolver = human_resolver();
    let allowed_team_ids: Option<BTreeSet<String>> = allowed.map(ids);
    let request = DelegatedTeamRequest {
        user_id: Some("human"),
        agent_id,
        team_id,
        explicit_team,
        allowed_team_ids: allowed_team_ids.as_ref(),
    };
    let outcome = resolve_delegated_agent_team(&access_resolver, &request).await;
    match expected {
        Ok(team) => assert_eq!(outcome.unwrap().as_deref(), team),
        Err(status) => {
            assert!(matches!(&outcome, Err(AgentAccessError::TeamNotGranted)));
            assert_eq!(rejected_status(outcome), status);
        }
    }
    assert_eq!(
        calls(&access_resolver.humans.calls),
        ["reload_user human fresh=true", "sources human allowed=None"]
    );
}

#[tokio::test]
async fn resolve_delegated_agent_team_without_a_user_is_rejected() {
    let access_resolver = human_resolver();
    let request = DelegatedTeamRequest {
        user_id: None,
        agent_id: "own",
        team_id: Some("team-1"),
        explicit_team: false,
        allowed_team_ids: None,
    };
    let outcome = resolve_delegated_agent_team(&access_resolver, &request).await;
    assert!(matches!(&outcome, Err(AgentAccessError::TeamNotGranted)));
}
