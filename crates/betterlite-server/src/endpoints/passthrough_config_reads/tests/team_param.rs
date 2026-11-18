#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;

use crate::endpoints::passthrough_config_reads::support::FakeConfig;
use crate::endpoints::passthrough_config_reads::support::FakeSettings;
use crate::endpoints::passthrough_config_reads::support::FakeTeams;
use crate::endpoints::passthrough_config_reads::support::SettingsReply::Missing;
use crate::endpoints::passthrough_config_reads::support::admin;
use crate::endpoints::passthrough_config_reads::support::call;
use crate::endpoints::passthrough_config_reads::support::endpoints_of;
use crate::endpoints::passthrough_config_reads::support::get;
use crate::endpoints::passthrough_config_reads::support::json;
use crate::endpoints::passthrough_config_reads::support::json_list;
use crate::endpoints::passthrough_config_reads::support::paths;
use crate::endpoints::passthrough_config_reads::support::read_app;

const TEAM: &str = "/config/pass_through_endpoint/team/t1";
const A: &str = r#"{"path": "/a", "target": "https://a"}"#;
const B: &str = r#"{"path": "/b", "target": "https://b"}"#;
const DB_A: &str = r#"{"id": "db-a", "path": "/db-a", "target": "https://db"}"#;
const DB_B: &str = r#"{"id": "db-b", "path": "/db-b", "target": "https://db"}"#;

#[tokio::test]
async fn the_team_path_filters_the_endpoints() {
    let teams = Arc::new(
        FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": ["/a", "/db-b"]}"#),
    );
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB_A, DB_B])));
    let config = Arc::new(FakeConfig::of(&[A, B]));
    let reply = call(
        read_app(&settings, &config, &teams),
        get(TEAM, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/a".to_owned(), "/db-b".to_owned()]);
    let endpoints = endpoints_of(&reply);
    assert!(endpoints[0].is_from_config);
    assert!(!endpoints[1].is_from_config);
}

#[tokio::test]
async fn an_unknown_team_in_the_path_answers_404() {
    let teams = Arc::new(FakeTeams::new());
    let settings = Arc::new(FakeSettings::refusing(Missing));
    let config = Arc::new(FakeConfig::of(&[A]));
    let reply = call(
        read_app(&settings, &config, &teams),
        get(TEAM, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.data(),
        json(r#"{"detail": {"error": "Team not found"}}"#)
    );
}

#[tokio::test]
async fn the_endpoint_id_query_narrows_the_team_read() {
    let teams = Arc::new(FakeTeams::new().with_team(
        "t1",
        r#"{"allowed_passthrough_routes": ["/db-a", "/db-b"]}"#,
    ));
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB_A, DB_B])));
    let config = Arc::new(FakeConfig::of(&[A]));
    let uri = format!("{TEAM}?endpoint_id=db-b");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let endpoints = endpoints_of(&reply);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].id.as_deref(), Some("db-b"));
}

#[tokio::test]
async fn a_team_read_without_a_caller_is_unauthorized() {
    let teams =
        Arc::new(FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": ["/a"]}"#));
    let settings = Arc::new(FakeSettings::refusing(Missing));
    let config = Arc::new(FakeConfig::of(&[A]));
    let reply = call(read_app(&settings, &config, &teams), get(TEAM, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(teams.looked_up().is_empty());
}

#[tokio::test]
async fn the_path_team_wins_over_a_query_team() {
    let teams = Arc::new(
        FakeTeams::new()
            .with_team("t1", r#"{"allowed_passthrough_routes": ["/a"]}"#)
            .with_team("t2", r#"{"allowed_passthrough_routes": ["/b"]}"#),
    );
    let settings = Arc::new(FakeSettings::refusing(Missing));
    let config = Arc::new(FakeConfig::of(&[A, B]));
    let uri = format!("{TEAM}?team_id=t2");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/a".to_owned()]);
    assert_eq!(teams.looked_up(), vec!["t1".to_owned()]);
}

#[tokio::test]
async fn a_team_allowed_no_routes_reads_no_endpoint() {
    let teams = Arc::new(FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": []}"#));
    let settings = Arc::new(FakeSettings::refusing(Missing));
    let config = Arc::new(FakeConfig::of(&[A]));
    let reply = call(
        read_app(&settings, &config, &teams),
        get(TEAM, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.data(), json(r#"{"endpoints": []}"#));
}
