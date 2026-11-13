use std::sync::Mutex;
use std::sync::atomic::Ordering;

use http::header::AUTHORIZATION;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::fixture::{FixedClock, Refused, Scripted, access};
use crate::proxy_auth::error::ProxyAuthError;
use crate::proxy_auth::{AccessToken, ProxyAuthHandler};

const NOW: i64 = 1_000;

fn handler(
    replies: Vec<Result<AccessToken, Refused>>,
) -> (ProxyAuthHandler<Scripted, FixedClock>, FixedClock) {
    let clock = FixedClock::at(NOW);
    let handler = ProxyAuthHandler {
        credential: Scripted::answering(replies),
        scope: "api://proxy/.default".to_owned(),
        clock: clock.clone(),
        cached_token: Mutex::new(None),
    };
    (handler, clock)
}

#[tokio::test]
async fn bearer_header_carries_the_token_value() {
    let (handler, _clock) = handler(vec![Ok(access("jwt-one", NOW + 3_600))]);
    let headers = handler.get_auth_headers().await.unwrap();
    let value = headers.get(AUTHORIZATION).unwrap();
    assert_eq!(value.to_str().unwrap(), "Bearer jwt-one");
    assert!(value.is_sensitive());
    assert_eq!(headers.len(), 1);
}

#[tokio::test]
async fn the_credential_receives_the_handler_scope() {
    let (handler, _clock) = handler(vec![Ok(access("jwt-one", NOW + 3_600))]);
    handler.get_token().await.unwrap();
    assert_eq!(
        *handler.credential.scopes.lock().unwrap(),
        ["api://proxy/.default"]
    );
}

#[rstest]
#[case::far_from_expiry(4_600, false)]
#[case::sixty_one_seconds_left(1_061, false)]
#[case::exactly_sixty_seconds_left(1_060, true)]
#[case::fifty_nine_seconds_left(1_059, true)]
#[case::expiring_now(1_000, true)]
#[case::already_expired(970, true)]
#[tokio::test]
async fn refreshes_only_within_sixty_seconds_of_expiry(
    #[case] expires_on: i64,
    #[case] refreshed: bool,
) {
    let (handler, _clock) = handler(vec![
        Ok(access("first", expires_on)),
        Ok(access("second", NOW + 3_600)),
    ]);
    let first = handler.get_token().await.unwrap();
    let again = handler.get_token().await.unwrap();
    assert_eq!(first.token.expose_secret(), "first");
    let expected = if refreshed { "second" } else { "first" };
    assert_eq!(again.token.expose_secret(), expected);
    let calls = handler.credential.calls.load(Ordering::SeqCst);
    assert_eq!(calls, if refreshed { 2 } else { 1 });
}

#[tokio::test]
async fn a_token_refreshes_once_time_passes_its_buffer() {
    let (handler, clock) = handler(vec![
        Ok(access("first", NOW + 3_600)),
        Ok(access("second", NOW + 7_200)),
    ]);
    handler.get_token().await.unwrap();
    clock.set(NOW + 3_539);
    assert_eq!(
        handler.get_token().await.unwrap().token.expose_secret(),
        "first"
    );
    clock.set(NOW + 3_540);
    assert_eq!(
        handler.get_token().await.unwrap().token.expose_secret(),
        "second"
    );
}

#[tokio::test]
async fn credential_errors_reach_the_caller_and_the_next_call_retries() {
    let (handler, _clock) = handler(vec![
        Err(Refused("idp down".to_owned())),
        Ok(access("recovered", NOW + 3_600)),
    ]);
    let failure = handler.get_auth_headers().await.unwrap_err();
    assert!(
        matches!(&failure, ProxyAuthError::Credential(Refused(reason)) if reason == "idp down")
    );
    let headers = handler.get_auth_headers().await.unwrap();
    assert_eq!(
        headers.get(AUTHORIZATION).unwrap().to_str().unwrap(),
        "Bearer recovered"
    );
}

#[tokio::test]
async fn a_failed_refresh_leaves_the_stale_token_unused() {
    let (handler, clock) = handler(vec![
        Ok(access("first", NOW + 100)),
        Err(Refused("idp down".to_owned())),
    ]);
    handler.get_token().await.unwrap();
    clock.set(NOW + 50);
    let failure = handler.get_token().await.unwrap_err();
    assert_eq!(failure.0, "idp down");
}

#[tokio::test]
async fn a_token_that_cannot_be_a_header_is_an_error() {
    let (handler, _clock) = handler(vec![Ok(access("bad\ntoken", NOW + 3_600))]);
    let failure = handler.get_auth_headers().await.unwrap_err();
    assert!(matches!(failure, ProxyAuthError::InvalidHeader(_)));
}
