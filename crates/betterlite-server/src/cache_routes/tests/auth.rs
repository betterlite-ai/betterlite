#![cfg(test)]

use axum::Router;
use axum::http::StatusCode;
use axum::middleware;
use betterlite_config::sections::auth_route_patterns::Settings;
use rstest::rstest;
use secrecy::SecretString;

use crate::auth::auth_route_patterns::{AuthSettings, stage};

use super::support_tests::{FakeLive, admin_router, call_text, working_cache};

fn protected(live: FakeLive) -> Router {
    let auth = AuthSettings {
        master_key: Some(SecretString::from("sk-master".to_owned())),
        routes: Settings {
            public_routes: Vec::new(),
        },
    };
    admin_router(live).layer(middleware::from_fn_with_state(auth, stage))
}

fn working() -> FakeLive {
    FakeLive::with(working_cache())
}

#[rstest]
#[case::ping("GET", "/cache/ping", "")]
#[case::delete("POST", "/cache/delete", r#"{"keys": ["k1"]}"#)]
#[case::redis_info("GET", "/cache/redis/info", "")]
#[case::flushall("POST", "/cache/flushall", "")]
#[tokio::test]
async fn every_route_refuses_a_request_without_a_key(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let (status, text) = call_text(protected(working()), method, uri, body, None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(text, "Internal Server Error");
}

#[rstest]
#[case::ping("GET", "/cache/ping", "")]
#[case::delete("POST", "/cache/delete", r#"{"keys": ["k1"]}"#)]
#[case::redis_info("GET", "/cache/redis/info", "")]
#[case::flushall("POST", "/cache/flushall", "")]
#[tokio::test]
async fn every_route_refuses_an_invalid_key(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let (status, text) = call_text(protected(working()), method, uri, body, Some("sk-wrong")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(text.contains("the API key is not valid"));
}

#[rstest]
#[case::ping("GET", "/cache/ping", "")]
#[case::delete("POST", "/cache/delete", r#"{"keys": ["k1"]}"#)]
#[case::redis_info("GET", "/cache/redis/info", "")]
#[case::flushall("POST", "/cache/flushall", "")]
#[tokio::test]
async fn every_route_runs_with_the_master_key(
    #[case] method: &str,
    #[case] uri: &str,
    #[case] body: &str,
) {
    let (status, _text) =
        call_text(protected(working()), method, uri, body, Some("sk-master")).await;
    assert_eq!(status, StatusCode::OK);
}
