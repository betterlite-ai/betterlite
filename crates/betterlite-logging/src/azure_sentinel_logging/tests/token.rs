use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{Reply, Rig, TOKEN_URL, rig};
use crate::azure_sentinel_logging::error::SentinelError;

async fn bearer(rig: &Rig) -> Result<String, SentinelError> {
    rig.logger
        .tokens
        .bearer(
            &rig.logger.settings,
            &*rig.logger.transport,
            &*rig.logger.clock,
        )
        .await
        .map(|token| token.expose_secret().to_owned())
}

#[tokio::test]
async fn the_first_request_posts_the_client_credentials_form() {
    let rig = rig(vec![Reply::token()], 512, 50_000);
    assert_eq!(bearer(&rig).await.expect("a token"), "token-1");
    let calls = rig.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, TOKEN_URL);
    assert_eq!(
        calls[0].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        calls[0].body,
        "client_id=client-1&client_secret=secret-1&scope=https%3A%2F%2Fmonitor.azure.com%2F.default&grant_type=client_credentials"
    );
}

#[tokio::test]
async fn a_cached_token_is_reused_until_sixty_seconds_before_expiry() {
    let rig = rig(
        vec![
            Reply::body(200, r#"{"access_token": "first", "expires_in": 3600}"#),
            Reply::body(200, r#"{"access_token": "second", "expires_in": 3600}"#),
        ],
        512,
        50_000,
    );
    assert_eq!(bearer(&rig).await.expect("a token"), "first");
    rig.clock.advance(3539);
    assert_eq!(bearer(&rig).await.expect("the cached token"), "first");
    assert_eq!(rig.transport.calls().len(), 1);
    rig.clock.advance(1);
    assert_eq!(bearer(&rig).await.expect("a refreshed token"), "second");
    assert_eq!(rig.transport.calls().len(), 2);
}

#[tokio::test]
async fn expires_in_sets_the_lifetime() {
    let rig = rig(
        vec![
            Reply::body(200, r#"{"access_token": "first", "expires_in": 100}"#),
            Reply::body(200, r#"{"access_token": "second", "expires_in": 100}"#),
        ],
        512,
        50_000,
    );
    assert_eq!(bearer(&rig).await.expect("a token"), "first");
    rig.clock.advance(39);
    assert_eq!(bearer(&rig).await.expect("the cached token"), "first");
    rig.clock.advance(1);
    assert_eq!(bearer(&rig).await.expect("a refreshed token"), "second");
}

#[tokio::test]
async fn a_missing_expires_in_means_3600_seconds() {
    let rig = rig(
        vec![
            Reply::body(200, r#"{"access_token": "first"}"#),
            Reply::body(200, r#"{"access_token": "second"}"#),
        ],
        512,
        50_000,
    );
    assert_eq!(bearer(&rig).await.expect("a token"), "first");
    rig.clock.advance(3539);
    assert_eq!(bearer(&rig).await.expect("the cached token"), "first");
    rig.clock.advance(1);
    assert_eq!(bearer(&rig).await.expect("a refreshed token"), "second");
}

#[tokio::test]
async fn a_status_other_than_200_is_an_error_that_is_not_cached() {
    let rig = rig(
        vec![Reply::body(401, "denied"), Reply::token()],
        512,
        50_000,
    );
    let error = bearer(&rig)
        .await
        .expect_err("the token request is refused");
    assert_eq!(
        error.to_string(),
        "Failed to get OAuth2 token: 401 - denied"
    );
    assert_eq!(bearer(&rig).await.expect("a token"), "token-1");
    assert_eq!(rig.transport.calls().len(), 2);
}

#[rstest]
#[case::absent(r#"{"expires_in": 3600}"#)]
#[case::empty(r#"{"access_token": "", "expires_in": 3600}"#)]
#[case::null(r#"{"access_token": null}"#)]
#[tokio::test]
async fn a_response_without_an_access_token_is_an_error(#[case] body: &str) {
    let rig = rig(vec![Reply::body(200, body)], 512, 50_000);
    let error = bearer(&rig).await.expect_err("there is no access token");
    assert_eq!(
        error.to_string(),
        "OAuth2 token response did not contain access_token"
    );
}

#[tokio::test]
async fn a_body_that_is_not_json_is_an_error() {
    let rig = rig(vec![Reply::body(200, "<html>")], 512, 50_000);
    assert!(matches!(
        bearer(&rig).await,
        Err(SentinelError::TokenBody(_))
    ));
}

#[tokio::test]
async fn a_network_failure_is_an_error() {
    let rig = rig(vec![Reply::Network], 512, 50_000);
    assert!(matches!(
        bearer(&rig).await,
        Err(SentinelError::Transport(_))
    ));
}
