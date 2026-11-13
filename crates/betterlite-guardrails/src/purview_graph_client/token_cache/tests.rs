#![cfg(test)]

use std::sync::Arc;
use std::thread;

use axum::http::StatusCode;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tokio::runtime::Builder;

use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::test_support::{
    MockReply, NOW_SECONDS, RecordedRequest, START_SECONDS, clock_at, guardrail, json_reply,
    standard_reply, start,
};
use crate::purview_graph_client::token_cache::get_access_token;

const TOKEN_PATH: &str = "/oauth2/v2.0/token";

#[tokio::test]
async fn token_cache_sends_the_client_credentials_form() {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);

    let token = get_access_token(&base).await.unwrap();

    assert_eq!(token.expose_secret(), "tok");
    let requests = mock.requests.lock().unwrap().clone();
    let request = requests.first().unwrap();
    assert_eq!(request.path, "/tenant-1/oauth2/v2.0/token");
    assert_eq!(
        request.headers.get("content-type").unwrap(),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(
        request.body,
        "grant_type=client_credentials&client_id=client-1&client_secret=secret-1&scope=https%3A%2F%2Fgraph.microsoft.com%2F.default"
    );
}

#[rstest]
#[case::many_seconds_left(100, 1)]
#[case::sixty_one_seconds_left(3539, 1)]
#[case::exactly_the_margin_left(3540, 2)]
#[case::expired(3601, 2)]
#[tokio::test]
async fn token_cache_reuses_a_token_until_the_sixty_second_margin(
    #[case] elapsed: u64,
    #[case] token_requests: usize,
) {
    let mock = start(standard_reply).await;
    let base = guardrail(&mock.base);
    get_access_token(&base).await.unwrap();

    NOW_SECONDS.set(START_SECONDS.saturating_add(elapsed));
    get_access_token(&base).await.unwrap();

    assert_eq!(mock.count(TOKEN_PATH), token_requests);
}

fn slow_token(_request: &RecordedRequest) -> MockReply {
    NOW_SECONDS.set(START_SECONDS.saturating_add(100));
    json_reply(r#"{"access_token":"slow","expires_in":3600}"#)
}

#[tokio::test]
async fn token_cache_counts_the_expiry_from_when_the_token_arrived() {
    let mock = start(slow_token).await;
    let base = guardrail(&mock.base);

    get_access_token(&base).await.unwrap();

    let expires_at = base
        .scope_and_token_cache_lock
        .lock()
        .unwrap()
        .oauth_token_cache
        .as_ref()
        .unwrap()
        .expires_at;
    assert_eq!(expires_at, clock_at(START_SECONDS.saturating_add(3700)));
}

fn token_without_lifetime(_request: &RecordedRequest) -> MockReply {
    json_reply(r#"{"access_token":"plain"}"#)
}

#[tokio::test]
async fn token_cache_defaults_the_lifetime_to_3599_seconds() {
    let mock = start(token_without_lifetime).await;
    let base = guardrail(&mock.base);

    get_access_token(&base).await.unwrap();

    let expires_at = base
        .scope_and_token_cache_lock
        .lock()
        .unwrap()
        .oauth_token_cache
        .as_ref()
        .unwrap()
        .expires_at;
    assert_eq!(expires_at, clock_at(START_SECONDS.saturating_add(3599)));
}

fn rejected_token(_request: &RecordedRequest) -> MockReply {
    MockReply {
        status: StatusCode::UNAUTHORIZED,
        headers: Vec::new(),
        body: "{}".to_owned(),
    }
}

#[tokio::test]
async fn token_cache_surfaces_a_rejected_token_request() {
    let mock = start(rejected_token).await;
    let base = guardrail(&mock.base);

    let error = get_access_token(&base).await.unwrap_err();

    assert!(matches!(
        error,
        PurviewGraphError::TokenStatus { status } if status.as_u16() == 401
    ));
    assert!(
        base.scope_and_token_cache_lock
            .lock()
            .unwrap()
            .oauth_token_cache
            .is_none()
    );
}

#[tokio::test]
async fn token_cache_is_shared_with_a_second_event_loop() {
    let mock = start(standard_reply).await;
    let base = Arc::new(guardrail(&mock.base));
    get_access_token(&base).await.unwrap();

    let other_loop_token = thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(get_access_token(&base)).unwrap()
            })
            .join()
            .unwrap()
    });

    assert_eq!(other_loop_token.expose_secret(), "tok");
    assert_eq!(mock.count(TOKEN_PATH), 1);
}
