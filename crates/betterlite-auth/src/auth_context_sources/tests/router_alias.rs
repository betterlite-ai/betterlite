use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::auth_context_sources::alias::AliasResolver;
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
const SAME_ALIAS: AliasResolver = |_, requested| Some(requested.to_owned());
const NO_ALIAS: AliasResolver = |_, _| None;

fn input(settings: Option<&Json>) -> RouterAliasInput<'_> {
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

#[rstest]
#[case::ineligible(RouterAliasInput { state: RouterAliasState::Ineligible { already_resolved: false }, ..input(None) }, RouterAliasRewrite::default())]
#[case::ineligible_marked(RouterAliasInput { state: RouterAliasState::Ineligible { already_resolved: true }, ..input(None) }, MARKED_ONLY)]
#[case::resolved(RouterAliasInput { state: RouterAliasState::Resolved, ..input(None) }, MARKED_ONLY)]
#[case::passthrough(RouterAliasInput { passthrough: true, ..input(None) }, MARKED_ONLY)]
#[case::missing_model(RouterAliasInput { requested_model: None, ..input(None) }, MARKED_ONLY)]
#[case::unavailable_body(RouterAliasInput { raw_body_available: false, ..input(None) }, MARKED_ONLY)]
#[tokio::test]
async fn router_marker_before_skip(
    #[case] mut request: RouterAliasInput<'static>,
    #[case] expected: RouterAliasRewrite,
) {
    request.team_id = Some("team-one");
    let teams = teams_with_settings(ALIASES);
    assert_eq!(
        resolve_router_alias(&request, &teams, |_, _| panic!(
            "skipped requests never resolve aliases"
        ))
        .await,
        expected
    );
    assert!(teams.calls().is_empty());
}

#[tokio::test]
async fn passthrough_not_rewritten() {
    let settings = json(ALIASES);
    let passthrough = teams_with_settings(ALIASES);
    let request_0 = RouterAliasInput {
        passthrough: true,
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    assert_eq!(
        resolve_router_alias(&request_0, &passthrough, group_alias).await,
        MARKED_ONLY
    );
    assert!(passthrough.calls().is_empty());

    // the same alias on a routed request_0 rewrites from the key settings alone
    let routed = teams_with_settings(ALIASES);
    let request_1 = RouterAliasInput {
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    assert_eq!(
        resolve_router_alias(&request_1, &routed, group_alias).await,
        RouterAliasRewrite {
            marker_set: true,
            client_requested_model: Some("alias-one".to_owned()),
            target: Some("group-one".to_owned()),
        }
    );
    assert!(routed.calls().is_empty());

    // with no key settings the team's settings answer
    let from_team = teams_with_settings(ALIASES);
    let request_2 = RouterAliasInput {
        team_id: Some("team-one"),
        ..input(None)
    };
    let rewrite = resolve_router_alias(&request_2, &from_team, group_alias).await;
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
        resolve_router_alias(&unavailable, &teams, group_alias).await,
        MARKED_ONLY
    );
    assert!(teams.calls().is_empty());

    let available = RouterAliasInput {
        team_id: Some("team-one"),
        ..input(Some(&settings))
    };
    let rewrite = resolve_router_alias(&available, &teams, group_alias).await;
    assert_eq!(rewrite.target, Some("group-one".to_owned()));
}

#[rstest]
#[case::same_name(SAME_ALIAS)]
#[case::no_target(NO_ALIAS)]
#[tokio::test]
async fn alias_self_none_unchanged(#[case] resolve: AliasResolver) {
    let settings = json(ALIASES);
    assert_eq!(
        resolve_router_alias(
            &input(Some(&settings)),
            &teams_with_settings(ALIASES),
            resolve
        )
        .await,
        MARKED_ONLY
    );
}

#[tokio::test]
async fn log_crlf_stripped() {
    let settings = json(r#"{"model_group_alias": {"bad\nname": "target\r\nname"}}"#);
    let (logs, _guard) = Logs::capture();
    let request_4 = RouterAliasInput {
        requested_model: Some("bad\nname"),
        ..input(Some(&settings))
    };
    let rewrite = resolve_router_alias(
        &request_4,
        &teams_with_settings(ALIASES),
        |aliases, requested| {
            assert_eq!(requested, "bad\nname");
            assert_eq!(aliases, Some(&json(r#"{"bad\nname":"target\r\nname"}"#)));
            Some("target\r\nname".to_owned())
        },
    )
    .await;
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
    let fresh = input(Some(&settings));
    assert_eq!(
        resolve_router_alias(&fresh, &teams, group_alias)
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
    let rewrite = resolve_router_alias(&earlier, &teams, |aliases, requested| {
        assert_eq!(requested, "alias-one");
        assert_eq!(
            aliases,
            Some(&json(r#"{"alias-one":{"model":"group-one"}}"#))
        );
        Some("group-one".to_owned())
    })
    .await;
    assert_eq!(rewrite.client_requested_model, Some("earlier".to_owned()));
    assert_eq!(rewrite.target, Some("group-one".to_owned()));

    // a request_4 with no rewrite leaves the scope key unset
    let unchanged = json(r#"{"model_group_alias": {"alias-one": "alias-one"}}"#);
    let request_5 = input(Some(&unchanged));
    assert_eq!(
        resolve_router_alias(&request_5, &teams, |_, requested| Some(
            requested.to_owned()
        ))
        .await
        .client_requested_model,
        None
    );
}

fn group_alias(aliases: Option<&Json>, requested: &str) -> Option<String> {
    assert_eq!(requested, "alias-one");
    assert_eq!(aliases, Some(&json(r#"{"alias-one":"group-one"}"#)));
    Some("group-one".to_owned())
}
