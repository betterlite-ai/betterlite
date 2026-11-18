#![cfg(test)]

use std::sync::Arc;

use axum::http::StatusCode;

use crate::endpoints::passthrough_config_reads::caller::CallerAccess::AdminView;
use crate::endpoints::passthrough_config_reads::caller::CallerAccess::FullAdmin;
use crate::endpoints::passthrough_config_reads::caller::CallerAccess::Limited;
use crate::endpoints::passthrough_config_reads::db_read::PASS_THROUGH_FIELD;
use crate::endpoints::passthrough_config_reads::support::FakeConfig;
use crate::endpoints::passthrough_config_reads::support::FakeSettings;
use crate::endpoints::passthrough_config_reads::support::FakeTeams;
use crate::endpoints::passthrough_config_reads::support::SettingsReply::Missing;
use crate::endpoints::passthrough_config_reads::support::SettingsReply::NotAllowed;
use crate::endpoints::passthrough_config_reads::support::admin;
use crate::endpoints::passthrough_config_reads::support::call;
use crate::endpoints::passthrough_config_reads::support::endpoints_of;
use crate::endpoints::passthrough_config_reads::support::get;
use crate::endpoints::passthrough_config_reads::support::json;
use crate::endpoints::passthrough_config_reads::support::json_list;
use crate::endpoints::passthrough_config_reads::support::master_key;
use crate::endpoints::passthrough_config_reads::support::member;
use crate::endpoints::passthrough_config_reads::support::paths;
use crate::endpoints::passthrough_config_reads::support::read_app;
use crate::endpoints::passthrough_config_reads::support::viewer;

const LIST: &str = "/config/pass_through_endpoint";
const C1: &str = r#"{"path": "/c1", "target": "https://config.example.com"}"#;
const C2: &str = r#"{"path": "/c2", "target": "https://config.example.com"}"#;
const DB1: &str = r#"{"id": "db-1", "path": "/db1", "target": "https://db.example.com"}"#;
const DB2: &str = r#"{"id": "db-2", "path": "/db2", "target": "https://db.example.com"}"#;
const SHARED_CONFIG: &str = r#"{"path": "/shared", "target": "https://config.example.com"}"#;
const SHARED_DB: &str =
    r#"{"id": "shared", "path": "/shared", "target": "https://db.example.com"}"#;
const INTERNAL: &str =
    r#"{"error": {"message": "Internal server error", "type": "internal_server_error"}}"#;

#[tokio::test]
async fn a_read_without_a_caller_is_unauthorized() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let reply = call(read_app(&settings, &config, &teams), get(LIST, None)).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    let error = reply.error();
    assert_eq!(error.message, "the API key is not valid");
    assert_eq!(error.kind, "auth_error");
    assert_eq!(error.param.as_deref(), Some("None"));
    assert_eq!(error.code.as_deref(), Some("401"));
    assert!(settings.called().is_empty());
    assert!(teams.looked_up().is_empty());
}

#[tokio::test]
async fn the_database_copy_wins_a_shared_path_and_config_only_endpoints_come_first() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[SHARED_DB, DB1])));
    let config = Arc::new(FakeConfig::of(&[C1, SHARED_CONFIG]));
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        paths(&reply),
        vec!["/c1".to_owned(), "/shared".to_owned(), "/db1".to_owned()]
    );
    let endpoints = endpoints_of(&reply);
    assert!(endpoints[0].is_from_config);
    assert!(!endpoints[1].is_from_config);
    assert_eq!(endpoints[1].target, "https://db.example.com");
    assert!(!endpoints[2].is_from_config);
}

#[tokio::test]
async fn an_endpoint_id_reads_only_the_database_endpoint() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1, DB2])));
    let config = Arc::new(FakeConfig::of(&[C1]));
    let teams = Arc::new(FakeTeams::new());
    let uri = format!("{LIST}?endpoint_id=db-2");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let endpoints = endpoints_of(&reply);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].id.as_deref(), Some("db-2"));
    assert_eq!(endpoints[0].path, "/db2");
    assert!(!endpoints[0].is_from_config);
}

#[tokio::test]
async fn an_unknown_endpoint_id_gives_an_empty_list() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1, DB2])));
    let config = Arc::new(FakeConfig::of(&[C1]));
    let teams = Arc::new(FakeTeams::new());
    let uri = format!("{LIST}?endpoint_id=missing");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.data(), json(r#"{"endpoints": []}"#));
}

#[tokio::test]
async fn a_team_query_filters_the_merged_endpoints() {
    let teams = Arc::new(
        FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": ["/c1", "/db1"]}"#),
    );
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1, DB2])));
    let config = Arc::new(FakeConfig::of(&[C1, C2]));
    let uri = format!("{LIST}?team_id=t1");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/c1".to_owned(), "/db1".to_owned()]);
}

#[tokio::test]
async fn the_team_filter_sees_the_database_copy_of_a_shared_path() {
    let teams = Arc::new(
        FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": ["/shared"]}"#),
    );
    let settings = Arc::new(FakeSettings::returning(json_list(&[SHARED_DB])));
    let config = Arc::new(FakeConfig::of(&[C1, SHARED_CONFIG]));
    let uri = format!("{LIST}?team_id=t1");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let endpoints = endpoints_of(&reply);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].id.as_deref(), Some("shared"));
    assert!(!endpoints[0].is_from_config);
    assert_eq!(endpoints[0].target, "https://db.example.com");
}

#[tokio::test]
async fn an_unknown_team_answers_404() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let uri = format!("{LIST}?team_id=missing");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.data(),
        json(r#"{"detail": {"error": "Team not found"}}"#)
    );
}

#[tokio::test]
async fn a_team_lookup_failure_answers_500() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::failing());
    let uri = format!("{LIST}?team_id=t1");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.data(), json(INTERNAL));
}

#[tokio::test]
async fn a_stored_field_that_is_not_a_list_answers_500() {
    let settings = Arc::new(FakeSettings::returning(json(r#""text""#)));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.data(), json(INTERNAL));
}

#[tokio::test]
async fn a_member_reads_the_config_endpoints_without_the_database() {
    let settings = Arc::new(FakeSettings::refusing(NotAllowed));
    let config = Arc::new(FakeConfig::of(&[C1]));
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&member())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/c1".to_owned()]);
    assert_eq!(
        settings.called(),
        vec![(PASS_THROUGH_FIELD.to_owned(), Limited)]
    );
}

#[tokio::test]
async fn a_viewer_reads_the_database_as_an_admin_view() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&viewer())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/db1".to_owned()]);
    assert_eq!(
        settings.called(),
        vec![(PASS_THROUGH_FIELD.to_owned(), AdminView)]
    );
}

#[tokio::test]
async fn a_master_key_reads_the_database_as_a_full_admin() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&master_key())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/db1".to_owned()]);
    assert_eq!(
        settings.called(),
        vec![(PASS_THROUGH_FIELD.to_owned(), FullAdmin)]
    );
}

#[tokio::test]
async fn an_empty_read_answers_an_empty_endpoint_list() {
    let settings = Arc::new(FakeSettings::refusing(Missing));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.data(), json(r#"{"endpoints": []}"#));
}

#[tokio::test]
async fn the_response_carries_every_declared_field() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[
        r#"{"id": "db-1", "path": "/db", "target": "https://db.example.com", "headers": {"x-api-key": "secret"}, "default_query_params": {"region": "eu"}, "include_subpath": true, "cost_per_request": 0.25, "timeout": 5, "auth": false, "guardrails": {"pii": {"request_fields": ["prompt"], "response_fields": ["text"]}}, "methods": ["GET", "POST"]}"#,
    ])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let reply = call(
        read_app(&settings, &config, &teams),
        get(LIST, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let expected = r#"{"endpoints": [{"id": "db-1", "path": "/db", "target": "https://db.example.com", "headers": {"x-api-key": "secret"}, "default_query_params": {"region": "eu"}, "include_subpath": true, "cost_per_request": 0.25, "timeout": 5.0, "auth": false, "guardrails": {"pii": {"request_fields": ["prompt"], "response_fields": ["text"]}}, "is_from_config": false, "methods": ["GET", "POST"]}]}"#;
    assert_eq!(reply.data(), json(expected));
}

#[tokio::test]
async fn a_settings_failure_with_an_endpoint_id_reads_no_endpoint() {
    let settings = Arc::new(FakeSettings::refusing(NotAllowed));
    let config = Arc::new(FakeConfig::of(&[C1]));
    let teams = Arc::new(FakeTeams::new());
    let uri = format!("{LIST}?endpoint_id=db-1");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.data(), json(r#"{"endpoints": []}"#));
}

#[tokio::test]
async fn a_missing_team_is_still_checked_when_the_endpoint_id_finds_nothing() {
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1])));
    let config = Arc::new(FakeConfig::none());
    let teams = Arc::new(FakeTeams::new());
    let uri = format!("{LIST}?endpoint_id=missing&team_id=missing");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.data(),
        json(r#"{"detail": {"error": "Team not found"}}"#)
    );
}

#[tokio::test]
async fn an_endpoint_id_and_a_team_filter_are_both_applied() {
    let teams =
        Arc::new(FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": ["/db1"]}"#));
    let settings = Arc::new(FakeSettings::returning(json_list(&[DB1, DB2])));
    let config = Arc::new(FakeConfig::of(&[C1]));
    let uri = format!("{LIST}?endpoint_id=db-1&team_id=t1");
    let reply = call(
        read_app(&settings, &config, &teams),
        get(&uri, Some(&admin())),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(paths(&reply), vec!["/db1".to_owned()]);
}
