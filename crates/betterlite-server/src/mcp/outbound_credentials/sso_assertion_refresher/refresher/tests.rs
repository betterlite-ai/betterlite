#![cfg(test)]

use std::sync::PoisonError;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::super::failure::{RefreshFailure, RefreshFailureKind};
use super::super::tests::support::{
    AccessFailure, CLIENT_ID, CLIENT_SECRET, NOW_SECONDS, RefresherFixture, TOKEN_ENDPOINT,
    access_with, assertion, basic_client, body, capture_logs, claims, id_token, outage,
    post_client, rejected, renewed_body, transport, unconfigured_client,
};
use super::super::transport::RefreshResponseBody;

const RENEWED_EXP: i64 = NOW_SECONDS + 3600;

#[tokio::test]
async fn rejects_an_assertion_without_a_refresh_token_before_it_reads_the_client() {
    let (logs, _guard) = capture_logs();
    let fixture = RefresherFixture::new(
        transport(Vec::new()),
        access_with(Vec::new(), AccessFailure::None),
        unconfigured_client(),
    );
    let expired = assertion(NOW_SECONDS, None);
    let failure = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect_err("a missing refresh token is a refusal");
    assert_eq!(failure.kind, RefreshFailureKind::Rejected);
    assert_eq!(failure.detail, "no refresh token was captured at sign-in");
    assert!(fixture.transport.sent().is_empty());
    let text = logs.text();
    assert!(text.contains("no refresh token was captured with it"));
    assert!(text.contains("offline_access"));
    assert!(!text.contains("GENERIC_CLIENT_ID"));
}

#[tokio::test]
async fn rejects_when_the_generic_sso_client_is_not_configured() {
    let (logs, _guard) = capture_logs();
    let fixture = RefresherFixture::new(
        transport(Vec::new()),
        access_with(Vec::new(), AccessFailure::None),
        unconfigured_client(),
    );
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let failure = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect_err("an unconfigured client is a refusal");
    assert_eq!(failure.kind, RefreshFailureKind::Rejected);
    assert_eq!(failure.detail, "the generic SSO client is not configured");
    assert!(fixture.transport.sent().is_empty());
    let text = logs.text();
    assert!(text.contains("the generic SSO client is not configured (GENERIC_TOKEN_ENDPOINT"));
    assert!(!text.contains("offline_access"));
}

#[tokio::test]
async fn posts_the_refresh_grant_with_basic_auth() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, None))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    let sent = fixture.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, TOKEN_ENDPOINT);
    assert_eq!(
        sent[0].form.get("grant_type").map(String::as_str),
        Some("refresh_token")
    );
    assert_eq!(
        sent[0].form.get("refresh_token").map(String::as_str),
        Some("carried-token")
    );
    assert_eq!(sent[0].form.len(), 2);
    assert_eq!(sent[0].headers.len(), 1);
    assert_eq!(
        sent[0].headers.get("Authorization").map(String::as_str),
        Some("Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ=")
    );
}

#[tokio::test]
async fn the_basic_auth_renewal_returns_and_stores_the_renewed_assertion() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, None))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    let renewed = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    assert_eq!(
        renewed.id_token.expose_secret(),
        id_token(&claims(RENEWED_EXP))
    );
    assert_eq!(
        renewed
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("carried-token")
    );
    let writes = fixture
        .access
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, "user-1");
}

#[tokio::test]
async fn posts_the_credentials_in_the_body_for_client_secret_post() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, None))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        post_client(),
    );
    fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    let sent = fixture.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].form.get("client_id").map(String::as_str),
        Some(CLIENT_ID)
    );
    assert_eq!(
        sent[0].form.get("client_secret").map(String::as_str),
        Some(CLIENT_SECRET)
    );
    assert_eq!(
        sent[0].form.get("refresh_token").map(String::as_str),
        Some("carried-token")
    );
    assert_eq!(sent[0].form.len(), 4);
    assert!(sent[0].headers.is_empty());
}

#[tokio::test]
async fn a_rotated_refresh_token_replaces_the_carried_one() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, Some("rotated-token")))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    let renewed = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    assert_eq!(
        renewed
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("rotated-token")
    );
    let writes = fixture
        .access
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0]
            .1
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("rotated-token")
    );
}

#[tokio::test]
async fn an_empty_refresh_token_keeps_the_carried_one() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, Some("")))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    let renewed = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    assert_eq!(
        renewed
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("carried-token")
    );
}

#[rstest]
#[case::absent(body(Vec::new()))]
#[case::empty_string(body(vec![("id_token", JsonValue::String(String::new()))]))]
#[case::not_a_string(body(vec![("id_token", JsonValue::from(5_i64))]))]
#[tokio::test]
async fn rejects_a_body_without_a_usable_id_token(#[case] answer: RefreshResponseBody) {
    let (logs, _guard) = capture_logs();
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(answer)]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    let failure = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect_err("no id_token is a refusal");
    assert_eq!(failure.kind, RefreshFailureKind::Rejected);
    assert_eq!(
        failure.detail,
        "the IdP's refresh response carried no usable id_token"
    );
    assert!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert!(logs.text().contains("returned no usable id_token"));
}

#[tokio::test]
async fn an_undecodable_id_token_logs_the_decoder_warning_too() {
    let (logs, _guard) = capture_logs();
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let answer = body(vec![(
        "id_token",
        JsonValue::String("not-a-three-segment-jwt".to_owned()),
    )]);
    let fixture = RefresherFixture::new(
        transport(vec![Ok(answer)]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    let failure = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect_err("an undecodable id_token is a refusal");
    assert_eq!(
        failure.detail,
        "the IdP's refresh response carried no usable id_token"
    );
    let text = logs.text();
    assert!(text.contains("SSO id_token could not be decoded"));
    assert!(text.contains("returned no usable id_token"));
}

#[tokio::test]
async fn keeps_a_newer_stored_row_and_returns_the_renewed_assertion() {
    let (logs, _guard) = capture_logs();
    let started = assertion(NOW_SECONDS, Some("carried-token"));
    let newer = assertion(NOW_SECONDS + 600, Some("newer-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, Some("rotated-token")))]),
        access_with(vec![("user-1", newer)], AccessFailure::None),
        basic_client(),
    );
    let renewed = fixture
        .refresher
        .refresh("user-1", &started)
        .await
        .expect("a skipped write-back still returns the renewal");
    assert_eq!(
        renewed.id_token.expose_secret(),
        id_token(&claims(RENEWED_EXP))
    );
    assert!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert!(logs.text().contains("keeping the stored one"));
}

#[tokio::test]
async fn writes_the_renewal_when_no_row_exists() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, None))]),
        access_with(Vec::new(), AccessFailure::None),
        basic_client(),
    );
    fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    let writes = fixture
        .access
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(writes.len(), 1);
}

#[tokio::test]
async fn writes_the_renewal_when_the_row_holds_the_same_id_token() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, None))]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect("the renewal succeeds");
    let writes = fixture
        .access
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].1.id_token.expose_secret(),
        id_token(&claims(RENEWED_EXP))
    );
}

#[rstest]
#[case::read(AccessFailure::Read)]
#[case::write(AccessFailure::Write)]
#[tokio::test]
async fn a_failing_write_back_is_unavailable_and_keeps_no_secret_in_the_logs(
    #[case] failure: AccessFailure,
) {
    let (logs, _guard) = capture_logs();
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Ok(renewed_body(RENEWED_EXP, Some("rotated-token")))]),
        access_with(Vec::new(), failure),
        basic_client(),
    );
    let refused = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect_err("an unrecorded renewal is transient");
    assert_eq!(refused.kind, RefreshFailureKind::Unavailable);
    assert_eq!(
        refused.detail,
        "the renewed IdP identity assertion could not be persisted"
    );
    assert!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    let text = logs.text();
    assert!(text.contains("could not persist the renewed IdP identity assertion"));
    assert!(!text.contains(CLIENT_SECRET));
    assert!(!text.contains("carried-token"));
    assert!(!text.contains("rotated-token"));
    assert!(!text.contains(&id_token(&claims(RENEWED_EXP))));
}

#[rstest]
#[case::rejected(rejected("the IdP refused the refresh with status 400"))]
#[case::outage(outage("the IdP token endpoint is unreachable"))]
#[tokio::test]
async fn returns_a_transport_failure_unchanged(#[case] answer: RefreshFailure) {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let fixture = RefresherFixture::new(
        transport(vec![Err(answer.clone())]),
        access_with(vec![("user-1", expired.clone())], AccessFailure::None),
        basic_client(),
    );
    let failure = fixture
        .refresher
        .refresh("user-1", &expired)
        .await
        .expect_err("the transport failed");
    assert_eq!(failure, answer);
    assert!(
        fixture
            .access
            .writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

#[test]
fn the_assertion_debug_redacts_both_tokens() {
    let expired = assertion(NOW_SECONDS, Some("carried-token"));
    let rendered = format!("{expired:?}");
    assert!(!rendered.contains("carried-token"));
    assert!(!rendered.contains(&id_token(&claims(NOW_SECONDS))));
    assert!(rendered.contains("id_token"));
}
