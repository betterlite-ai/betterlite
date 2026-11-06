use std::sync::Mutex;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::fixture::{FixedClock, TokenServer, client, serve};
use crate::proxy_auth::error::OAuth2Error;
use crate::proxy_auth::{GenericOAuth2Credential, TokenCredential as _};

const NOW: i64 = 1_000;

fn credential(server: &TokenServer, clock: &FixedClock) -> GenericOAuth2Credential<FixedClock> {
    GenericOAuth2Credential {
        client_id: "my-client".to_owned(),
        client_secret: SecretString::from("s3cr3t value"),
        token_url: server.url.clone(),
        client: client(),
        clock: clock.clone(),
        cached_token: Mutex::new(None),
    }
}

fn requests(server: &TokenServer) -> Vec<String> {
    server.requests.lock().unwrap().clone()
}

#[tokio::test]
async fn posts_the_client_credentials_grant_as_a_form() {
    let server = serve(200, r#"{"access_token":"jwt-one","expires_in":120}"#).await;
    let credential = credential(&server, &FixedClock::at(NOW));
    credential.get_token("api://x/.default").await.unwrap();
    let sent = requests(&server);
    let (head, body) = sent[0].split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /oauth2/token HTTP/1.1"));
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: application/x-www-form-urlencoded")
    );
    assert_eq!(
        body,
        "grant_type=client_credentials&client_id=my-client&client_secret=s3cr3t+value&scope=api%3A%2F%2Fx%2F.default"
    );
}

#[rstest]
#[case::missing_expires_in(r#"{"access_token":"jwt"}"#, NOW + 3_600)]
#[case::given_expires_in(r#"{"access_token":"jwt","expires_in":120}"#, NOW + 120)]
#[case::explicit_zero_is_used(r#"{"access_token":"jwt","expires_in":0}"#, NOW)]
#[case::extra_fields_are_ignored(r#"{"access_token":"jwt","token_type":"Bearer","expires_in":5}"#, NOW + 5)]
#[tokio::test]
async fn expiry_is_the_clock_plus_expires_in(#[case] body: &str, #[case] expires_on: i64) {
    let server = serve(200, body).await;
    let token = credential(&server, &FixedClock::at(NOW))
        .get_token("scope")
        .await
        .unwrap();
    assert_eq!(token.token.expose_secret(), "jwt");
    assert_eq!(token.expires_on, expires_on);
}

#[tokio::test]
async fn the_cached_token_is_reused_for_every_scope() {
    let server = serve(200, r#"{"access_token":"jwt-one","expires_in":3600}"#).await;
    let credential = credential(&server, &FixedClock::at(NOW));
    let first = credential.get_token("scope-a").await.unwrap();
    let second = credential.get_token("scope-b").await.unwrap();
    assert_eq!(first.token.expose_secret(), second.token.expose_secret());
    assert_eq!(requests(&server).len(), 1);
    assert!(requests(&server)[0].contains("scope=scope-a"));
}

#[rstest]
#[case::sixty_one_seconds_left(4_539, 1)]
#[case::exactly_sixty_seconds_left(4_540, 2)]
#[case::expired(4_700, 2)]
#[tokio::test]
async fn the_token_is_exchanged_again_inside_the_buffer(#[case] later: i64, #[case] posts: usize) {
    let server = serve(200, r#"{"access_token":"jwt-one","expires_in":3600}"#).await;
    let clock = FixedClock::at(NOW);
    let credential = credential(&server, &clock);
    credential.get_token("scope").await.unwrap();
    clock.set(later);
    credential.get_token("scope").await.unwrap();
    assert_eq!(requests(&server).len(), posts);
}

#[rstest]
#[case::unauthorized(401)]
#[case::server_error(500)]
#[case::redirect(302)]
#[tokio::test]
async fn a_status_outside_2xx_is_an_error_and_caches_nothing(#[case] status: u16) {
    let server = serve(status, r#"{"access_token":"jwt","expires_in":3600}"#).await;
    let credential = credential(&server, &FixedClock::at(NOW));
    let failure = credential.get_token("scope").await.unwrap_err();
    assert!(matches!(failure, OAuth2Error::Status { status: seen } if seen == status));
    credential.get_token("scope").await.unwrap_err();
    assert_eq!(requests(&server).len(), 2);
}

#[rstest]
#[case::no_access_token(r#"{"expires_in":3600}"#)]
#[case::not_json("<html>")]
#[case::access_token_not_text(r#"{"access_token":5}"#)]
#[case::expires_in_not_a_number(r#"{"access_token":"jwt","expires_in":"soon"}"#)]
#[case::null_expires_in(r#"{"access_token":"jwt","expires_in":null}"#)]
#[tokio::test]
async fn a_body_that_is_not_a_token_response_is_an_error(#[case] body: &str) {
    let server = serve(200, body).await;
    let failure = credential(&server, &FixedClock::at(NOW))
        .get_token("scope")
        .await
        .unwrap_err();
    assert!(matches!(failure, OAuth2Error::Body(_)));
}

#[tokio::test]
async fn an_expiry_past_the_timestamp_range_is_an_error() {
    let server = serve(
        200,
        r#"{"access_token":"jwt","expires_in":9223372036854775807}"#,
    )
    .await;
    let failure = credential(&server, &FixedClock::at(NOW))
        .get_token("scope")
        .await
        .unwrap_err();
    assert!(matches!(failure, OAuth2Error::ExpiryOverflow));
}

#[tokio::test]
async fn an_unreachable_endpoint_is_a_transport_error() {
    let server = serve(200, "{}").await;
    let mut credential = credential(&server, &FixedClock::at(NOW));
    credential.token_url = "http://127.0.0.1:1/oauth2/token".parse().unwrap();
    let failure = credential.get_token("scope").await.unwrap_err();
    assert!(matches!(failure, OAuth2Error::Transport(_)));
}
