use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::auth_context_sources::router_alias::{
    RouterAliasInput, RouterAliasRewrite, RouterAliasState, resolve_router_alias,
};

use super::support::{FakeTeams, Logs, TeamAnswer, json, team_columns};

const ALIASES: &str = r#"{"model_group_alias": {"alias-one": "group-one"}}"#;
const MARKED_ONLY: RouterAliasRewrite = RouterAliasRewrite {
    marker_set: true,
    client_requested_model: None,
    target: None,
};

fn input<'request>(settings: Option<&'request Json>) -> RouterAliasInput<'request> {
    RouterAliasInput {
        state: RouterAliasState::Fresh,
        passthrough: false,
        requested_model: Some("alias-one"),
        raw_body_available: true,
        key_router_settings: settings,
        team_id: None,
        client_requested_model: None,
    }
}

fn teams_with_settings(settings: &str) -> FakeTeams {
    FakeTeams::new(TeamAnswer::Settings(team_columns(settings)))
}

#[tokio::test]
async fn router_marker_before_skip() {
    let teams = teams_with_settings(ALIASES);
    let outside_route = RouterAliasInput {
        state: RouterAliasState::Ineligible {
            already_resolved: false,
        },
        team_id: Some("team-one"),
        ..input(None)
    };
    assert_eq!(
        resolve_router_alias(&outside_route, &teams).await,
        RouterAliasRewrite::default()
    );

    // an ineligible route reports the marker it found, and a marked one keeps it
    let already_marked = RouterAliasInput {
        state: RouterAliasState::Ineligible {
            already_resolved: true,
        },
        team_id: Some("team-one"),
        ..input(None)
    };
    assert_eq!(
        resolve_router_alias(&already_marked, &teams).await,
        MARKED_ONLY
    );

    // a marked request never reaches the settings read again
    let marked = RouterAliasInput {
        state: RouterAliasState::Resolved,
        team_id: Some("team-one"),
        ..input(None)
    };
    assert_eq!(resolve_router_alias(&marked, &teams).await, MARKED_ONLY);

    // every later early return still sets the marker, so a retry cannot rewrite
    let passthrough = RouterAliasInput {
        passthrough: true,
        team_id: Some("team-one"),
        ..input(None)
    };
    assert_eq!(
        resolve_router_alias(&passthrough, &teams).await,
        MARKED_ONLY
    );
    let missing_model = RouterAliasInput {
        requested_model: None,
        team_id: Some("team-one"),
        ..input(None)
    };
    assert_eq!(
        resolve_router_alias(&missing_model, &teams).await,
        MARKED_ONLY
    );
    let unavailable_body = RouterAliasInput {
        raw_body_available: false,
        team_id: Some("team-one"),
        ..input(None)
    };
    assert_eq!(
        resolve_router_alias(&unavailable_body, &teams).await,
        MARKED_ONLY
    );
    assert!(teams.calls().is_empty());
}

#[tokio::test]
async fn passthrough_not_rewritten() {
    let settings = json(ALIASES);
    let passthrough = teams_with_settings(ALIASES);
    let request = RouterAliasInput {
        passthrough: true,
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    assert_eq!(
        resolve_router_alias(&request, &passthrough).await,
        MARKED_ONLY
    );
    assert!(passthrough.calls().is_empty());

    // the same alias on a routed request rewrites from the key settings alone
    let routed = teams_with_settings(ALIASES);
    let request = RouterAliasInput {
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    assert_eq!(
        resolve_router_alias(&request, &routed).await,
        RouterAliasRewrite {
            marker_set: true,
            client_requested_model: Some("alias-one".to_owned()),
            target: Some("group-one".to_owned()),
        }
    );
    assert!(routed.calls().is_empty());

    // with no key settings the team's settings answer
    let from_team = teams_with_settings(ALIASES);
    let request = RouterAliasInput {
        team_id: Some("team-one"),
        ..input(None)
    };
    let rewrite = resolve_router_alias(&request, &from_team).await;
    assert_eq!(rewrite.target, Some("group-one".to_owned()));
    assert_eq!(from_team.calls(), vec!["team-one".to_owned()]);
}

#[tokio::test]
async fn nonjson_not_rewritten() {
    let settings = json(ALIASES);
    let teams = teams_with_settings(ALIASES);
    let unavailable = RouterAliasInput {
        raw_body_available: false,
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    assert_eq!(
        resolve_router_alias(&unavailable, &teams).await,
        MARKED_ONLY
    );
    assert!(teams.calls().is_empty());

    let available = RouterAliasInput {
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    let rewrite = resolve_router_alias(&available, &teams).await;
    assert_eq!(rewrite.target, Some("group-one".to_owned()));
}

#[rstest]
#[case::same_name(json(r#"{"model_group_alias": {"alias-one": "alias-one"}}"#))]
#[case::empty_target(json(r#"{"model_group_alias": {"alias-one": ""}}"#))]
#[case::number_target(json(r#"{"model_group_alias": {"alias-one": 7}}"#))]
#[case::object_target(json(r#"{"model_group_alias": {"alias-one": {}}}"#))]
#[case::object_empty_model(json(r#"{"model_group_alias": {"alias-one": {"model": ""}}}"#))]
#[case::other_model(json(r#"{"model_group_alias": {"other-model": "group-one"}}"#))]
#[case::no_alias_map(json(r#"{"num_retries": 3}"#))]
#[tokio::test]
async fn alias_self_none_unchanged(#[case] candidate: Json) {
    let request = RouterAliasInput {
        requested_model: Some("alias-one"),
        ..input(Some(&candidate))
    };
    assert_eq!(
        resolve_router_alias(&request, &teams_with_settings(ALIASES)).await,
        MARKED_ONLY,
        "{candidate:?}"
    );
}

#[tokio::test]
async fn log_crlf_stripped() {
    let settings = json(r#"{"model_group_alias": {"bad\nname": "target\r\nname"}}"#);
    let (logs, _guard) = Logs::capture();
    let request = RouterAliasInput {
        requested_model: Some("bad\nname"),
        ..input(Some(&settings))
    };
    let rewrite = resolve_router_alias(&request, &teams_with_settings(ALIASES)).await;
    assert_eq!(rewrite.target, Some("target\r\nname".to_owned()));
    let captured = logs.text();
    assert!(
        captured.contains("resolved badname -> targetname before auth"),
        "{captured}"
    );
    assert!(!captured.contains("bad\nname"), "{captured}");
}

#[tokio::test]
async fn client_model_setdefault() {
    let teams = teams_with_settings(ALIASES);
    let settings = json(ALIASES);
    let fresh = RouterAliasInput {
        ..input(Some(&settings))
    };
    assert_eq!(
        resolve_router_alias(&fresh, &teams)
            .await
            .client_requested_model,
        Some("alias-one".to_owned())
    );

    // a value already in the scope key wins over the requested model
    let object_shape = json(r#"{"model_group_alias": {"alias-one": {"model": "group-one"}}}"#);
    let earlier = RouterAliasInput {
        client_requested_model: Some("earlier"),
        ..input(Some(&object_shape))
    };
    let rewrite = resolve_router_alias(&earlier, &teams).await;
    assert_eq!(rewrite.client_requested_model, Some("earlier".to_owned()));
    assert_eq!(rewrite.target, Some("group-one".to_owned()));

    // a request with no rewrite leaves the scope key unset
    let unchanged = json(r#"{"model_group_alias": {"alias-one": "alias-one"}}"#);
    let request = RouterAliasInput {
        ..input(Some(&unchanged))
    };
    assert_eq!(
        resolve_router_alias(&request, &teams)
            .await
            .client_requested_model,
        None
    );
}
