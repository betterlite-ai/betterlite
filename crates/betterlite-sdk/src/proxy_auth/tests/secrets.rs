use std::error::Error as _;
use std::sync::Mutex;

use secrecy::SecretString;

use super::fixture::{FixedClock, Scripted, access, client, serve};
use crate::proxy_auth::{GenericOAuth2Credential, ProxyAuthHandler, TokenCredential as _};

const NOW: i64 = 1_000;

#[test]
fn an_access_token_debug_hides_the_token() {
    let shown = format!("{:?}", access("jwt-secret-value", 5));
    assert!(!shown.contains("jwt-secret-value"));
    assert!(shown.contains("expires_on: 5"));
}

#[tokio::test]
async fn debug_of_the_credential_and_handler_hides_every_secret() {
    let server = serve(
        200,
        r#"{"access_token":"jwt-secret-value","expires_in":3600}"#,
    )
    .await;
    let credential = GenericOAuth2Credential {
        client_id: "my-client".to_owned(),
        client_secret: SecretString::from("client-secret-value"),
        token_url: server.url.clone(),
        client: client(),
        clock: FixedClock::at(NOW),
        cached_token: Mutex::new(None),
    };
    credential.get_token("scope").await.unwrap();
    let shown = format!("{credential:?}");
    assert!(!shown.contains("client-secret-value"));
    assert!(!shown.contains("jwt-secret-value"));
    assert!(shown.contains("my-client"));
    let handler = ProxyAuthHandler {
        credential: Scripted::default(),
        scope: "scope".to_owned(),
        clock: FixedClock::at(NOW),
        cached_token: Mutex::new(Some(access("jwt-secret-value", 5))),
    };
    assert!(!format!("{handler:?}").contains("jwt-secret-value"));
}

#[tokio::test]
async fn errors_and_their_sources_never_carry_the_client_secret() {
    let server = serve(401, r#"{"error":"client-secret-value"}"#).await;
    let credential = GenericOAuth2Credential {
        client_id: "my-client".to_owned(),
        client_secret: SecretString::from("client-secret-value"),
        token_url: server.url.clone(),
        client: client(),
        clock: FixedClock::at(NOW),
        cached_token: Mutex::new(None),
    };
    let failure = credential.get_token("scope").await.unwrap_err();
    let mut parts = vec![format!("{failure} {failure:?}")];
    let mut source = failure.source();
    while let Some(cause) = source {
        parts.push(format!("{cause} {cause:?}"));
        source = cause.source();
    }
    let text = parts.join(" ");
    assert!(!text.contains("client-secret-value"));
}
