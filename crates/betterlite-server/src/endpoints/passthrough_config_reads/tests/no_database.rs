#![cfg(test)]

use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use crate::endpoints::passthrough_config_reads::support::FakeConfig;
use crate::endpoints::passthrough_config_reads::support::FakeTeams;
use crate::endpoints::passthrough_config_reads::support::admin;
use crate::endpoints::passthrough_config_reads::support::app;
use crate::endpoints::passthrough_config_reads::support::call;
use crate::endpoints::passthrough_config_reads::support::configured;
use crate::endpoints::passthrough_config_reads::support::get;
use crate::endpoints::passthrough_config_reads::support::json;
use crate::endpoints::passthrough_config_reads::support::team_lookup;

const LIST: &str = "/config/pass_through_endpoint";
const TEAM: &str = "/config/pass_through_endpoint/team/missing";

fn app_without_a_database() -> (Router, Arc<FakeTeams>) {
    let config = Arc::new(FakeConfig::of(&[
        r#"{"path": "/a", "target": "https://a"}"#,
    ]));
    let teams =
        Arc::new(FakeTeams::new().with_team("t1", r#"{"allowed_passthrough_routes": ["/a"]}"#));
    let router = app(None, configured(&config), team_lookup(&teams));
    (router, teams)
}

fn expected_body() -> String {
    let message = CommonProxyErrors::DbNotConnectedError.as_str();
    format!(r#"{{"detail": {{"error": "{message}"}}}}"#)
}

#[tokio::test]
async fn the_list_route_answers_500_without_a_database() {
    let (router, _teams) = app_without_a_database();
    let reply = call(router, get(LIST, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail_error(),
        CommonProxyErrors::DbNotConnectedError.as_str()
    );
    assert_eq!(reply.data(), json(&expected_body()));
}

#[tokio::test]
async fn the_team_route_answers_500_without_a_database() {
    let (router, teams) = app_without_a_database();
    let reply = call(router, get(TEAM, Some(&admin()))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.data(), json(&expected_body()));
    assert!(teams.looked_up().is_empty());
}
