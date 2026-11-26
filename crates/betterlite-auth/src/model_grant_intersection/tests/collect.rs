use std::collections::BTreeSet;

use rstest::rstest;

use crate::model_grant_intersection::collect::{MatchedRequest, collect_matched_groups};
use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::tests::support::{
    Effect, World, grant_token, group_set, list, membership_row, owned, registry_failure, team_row,
    text,
};
use crate::model_grant_intersection::token::GrantToken;

fn collect_request<'request>(
    world: &'request World,
    token: Option<&'request GrantToken>,
    models: Option<&'request [String]>,
) -> MatchedRequest<'request> {
    MatchedRequest {
        models,
        token,
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(world),
        sources: Some(world),
    }
}

#[tokio::test]
async fn layered_grant_intersection() {
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a", "tier-b"]))))
        .with_serving(&[("gpt-4o", &["tier-a", "tier-b"])]);
    let token = grant_token(&[("models", list(&["tier-a"])), ("team_id", text("team-1"))]);
    let team = team_row("team-1", &["tier-b"], None);
    let models = owned(&["gpt-4o"]);
    let matched = collect_matched_groups(MatchedRequest {
        team: Some(&team),
        ..collect_request(&world, Some(&token), Some(&models))
    })
    .await
    .unwrap();
    assert_eq!(matched, owned(&["tier-a", "tier-b"]));
}

#[tokio::test]
async fn member_narrowing() {
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["claude-tier"]))))
        .with_serving(&[("claude-sonnet", &["claude-tier"])])
        .with_membership(Ok(Some(membership_row(
            "u1",
            "team-1",
            Some(&["claude-tier"]),
        ))));
    let token = grant_token(&[
        ("models", list(&["*"])),
        ("team_id", text("team-1")),
        ("user_id", text("u1")),
    ]);
    let team = team_row("team-1", &["*"], None);
    let models = owned(&["claude-sonnet"]);
    let matched = collect_matched_groups(MatchedRequest {
        team: Some(&team),
        ..collect_request(&world, Some(&token), Some(&models))
    })
    .await
    .unwrap();
    assert_eq!(matched, owned(&["claude-tier"]));
}

#[tokio::test]
async fn missing_collection_inputs() {
    let world = World::new()
        .with_registry(Ok(None))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let token = grant_token(&[("models", list(&["tier-a"]))]);
    let models = owned(&["gpt-4o"]);
    for case in [
        MatchedRequest {
            models: None,
            ..collect_request(&world, Some(&token), Some(&models))
        },
        MatchedRequest {
            token: None,
            ..collect_request(&world, Some(&token), Some(&models))
        },
        MatchedRequest {
            groups: None,
            ..collect_request(&world, Some(&token), Some(&models))
        },
        MatchedRequest {
            sources: None,
            ..collect_request(&world, Some(&token), Some(&models))
        },
    ] {
        let matched = collect_matched_groups(case).await.unwrap();
        assert!(matched.is_empty());
    }
    assert!(world.effects().is_empty());
}

#[tokio::test]
async fn empty_registry_short_circuit() {
    let world = World::new()
        .with_registry(Ok(Some(BTreeSet::new())))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let token = grant_token(&[("models", list(&["tier-a"]))]);
    let models = owned(&["gpt-4o"]);
    let matched = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap();
    assert!(matched.is_empty());
    assert_eq!(world.effects(), vec![Effect::Registry]);
}

#[tokio::test]
async fn unknown_registry_all_covering() {
    let world = World::new()
        .with_registry(Ok(None))
        .with_serving(&[("gpt-4o", &["tier-a", "tier-b", "free-tier"])]);
    let token = grant_token(&[("models", list(&["tier-a"]))]);
    let models = owned(&["gpt-4o"]);
    let matched = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap();
    assert_eq!(matched, owned(&["tier-a"]));
}

#[tokio::test]
async fn team_scoped_group_union() {
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["g1", "g2"]))))
        .with_serving(&[("m1", &["g1"]), ("m2", &["g2"])]);
    let token = grant_token(&[("models", list(&["g1", "g2"])), ("team_id", text("team-1"))]);
    let models = owned(&["m1", "m2"]);
    let matched = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap();
    assert_eq!(matched, owned(&["g1", "g2"]));
    assert_eq!(
        world.effects(),
        vec![
            Effect::Registry,
            Effect::Serving {
                model: "m1".to_owned(),
                team_id: Some("team-1".to_owned()),
            },
            Effect::Serving {
                model: "m2".to_owned(),
                team_id: Some("team-1".to_owned()),
            },
        ]
    );
}

#[tokio::test]
async fn group_name_must_be_granted() {
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let token = grant_token(&[("models", list(&["gpt-4o"]))]);
    let models = owned(&["gpt-4o"]);
    let matched = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap();
    assert!(matched.is_empty());
}

#[rstest]
#[case::star(&["*"])]
#[case::empty(&[])]
#[case::all_proxy_models(&["all-proxy-models"])]
#[tokio::test]
async fn wildcard_no_attribution(#[case] key_models: &[&str]) {
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let token = grant_token(&[("models", list(key_models))]);
    let models = owned(&["gpt-4o"]);
    let matched = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap();
    assert!(matched.is_empty());
}

#[tokio::test]
async fn matched_sorted_deduplicated() {
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a", "tier-b"]))))
        .with_serving(&[("gpt-4o", &["tier-b", "tier-a"])]);
    let token = grant_token(&[("models", list(&["tier-b", "tier-a"]))]);
    let team = team_row("team-1", &["tier-a"], None);
    let models = owned(&["gpt-4o"]);
    let matched = collect_matched_groups(MatchedRequest {
        team: Some(&team),
        ..collect_request(&world, Some(&token), Some(&models))
    })
    .await
    .unwrap();
    assert_eq!(matched, owned(&["tier-a", "tier-b"]));
}

#[tokio::test]
async fn collection_errors_escape() {
    let token = grant_token(&[("models", list(&["tier-a"]))]);
    let models = owned(&["gpt-4o"]);

    let world = World::new().with_registry(Err(registry_failure()));
    let error = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap_err();
    assert!(matches!(error, IntersectionError::Registry(_)));
    assert_eq!(error.http_status(), None);

    let world = World::new().with_registry(Ok(None)).with_broken_router();
    let error = collect_matched_groups(collect_request(&world, Some(&token), Some(&models)))
        .await
        .unwrap_err();
    assert!(matches!(error, IntersectionError::Groups(_)));
    assert_eq!(error.http_status(), None);
}
