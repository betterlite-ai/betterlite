#![cfg(test)]

use rstest::rstest;

use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::endpoints::passthrough_config_reads::support::FakeTeams;
use crate::endpoints::passthrough_config_reads::support::endpoint;
use crate::endpoints::passthrough_config_reads::team_filter::TeamFilterError;
use crate::endpoints::passthrough_config_reads::team_filter::filter_endpoints_by_team;

fn endpoints() -> Vec<PassThroughGenericEndpoint> {
    vec![endpoint("/a"), endpoint("/b"), endpoint("/c")]
}

fn allowed(metadata: &str) -> FakeTeams {
    FakeTeams::new().with_team("t1", metadata)
}

#[tokio::test]
async fn a_missing_team_is_an_error() {
    let teams = FakeTeams::new();
    let error = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect_err("no team");
    assert!(matches!(error, TeamFilterError::TeamNotFound));
}

#[tokio::test]
async fn a_team_with_null_metadata_allows_every_endpoint() {
    let teams = allowed("null");
    let read = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("every endpoint");
    assert_eq!(read, endpoints());
}

#[rstest]
#[case::empty_object("{}")]
#[case::other_keys(r#"{"other": true}"#)]
#[tokio::test]
async fn a_team_without_the_allowed_routes_key_allows_every_endpoint(#[case] metadata: &str) {
    let teams = allowed(metadata);
    let read = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("every endpoint");
    assert_eq!(read, endpoints());
}

#[tokio::test]
async fn a_null_allowed_routes_value_allows_every_endpoint() {
    let teams = allowed(r#"{"allowed_passthrough_routes": null}"#);
    let read = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("every endpoint");
    assert_eq!(read, endpoints());
}

#[tokio::test]
async fn the_allowed_routes_filter_the_endpoints() {
    let teams = allowed(r#"{"allowed_passthrough_routes": ["/b"]}"#);
    let read = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("the allowed endpoint");
    assert_eq!(read, vec![endpoint("/b")]);
}

#[tokio::test]
async fn an_empty_allowed_routes_list_allows_no_endpoint() {
    let teams = allowed(r#"{"allowed_passthrough_routes": []}"#);
    let read = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("no endpoint");
    assert!(read.is_empty());
}

#[tokio::test]
async fn a_non_string_allowed_route_is_ignored() {
    let teams = allowed(r#"{"allowed_passthrough_routes": ["/b", 7, null, false, [], {}]}"#);
    let read = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("the allowed endpoint");
    assert_eq!(read, vec![endpoint("/b")]);
}

#[tokio::test]
async fn an_allowed_route_matches_the_whole_path() {
    let teams = allowed(r#"{"allowed_passthrough_routes": ["/b"]}"#);
    let declared = vec![endpoint("/b"), endpoint("/b/c"), endpoint("/bc")];
    let read = filter_endpoints_by_team(&teams, "t1", &declared)
        .await
        .expect("the exact path");
    assert_eq!(read, vec![endpoint("/b")]);
}

#[rstest]
#[case::text(r#"{"allowed_passthrough_routes": "/b"}"#)]
#[case::object(r#"{"allowed_passthrough_routes": {"path": "/b"}}"#)]
#[case::number(r#"{"allowed_passthrough_routes": 3}"#)]
#[case::flag(r#"{"allowed_passthrough_routes": true}"#)]
#[tokio::test]
async fn a_non_list_allowed_routes_value_is_an_error(#[case] metadata: &str) {
    let teams = allowed(metadata);
    let error = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect_err("not a list");
    assert!(matches!(error, TeamFilterError::AllowedRoutes { .. }));
}

#[rstest]
#[case::text(r#""text""#)]
#[case::number("3")]
#[case::flag("true")]
#[case::list(r#"["/a"]"#)]
#[tokio::test]
async fn metadata_that_is_not_an_object_is_an_error(#[case] metadata: &str) {
    let teams = allowed(metadata);
    let error = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect_err("not an object");
    assert!(matches!(error, TeamFilterError::Metadata { .. }));
}

#[tokio::test]
async fn a_team_lookup_failure_is_an_error() {
    let teams = FakeTeams::failing();
    let error = filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect_err("the lookup fails");
    assert!(matches!(error, TeamFilterError::Lookup(_)));
}

#[tokio::test]
async fn the_team_is_looked_up_by_its_id() {
    let teams = allowed("{}");
    filter_endpoints_by_team(&teams, "t1", &endpoints())
        .await
        .expect("every endpoint");
    assert_eq!(teams.looked_up(), vec!["t1".to_owned()]);
}
