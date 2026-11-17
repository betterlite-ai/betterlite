use std::time::Duration;

use secrecy::ExposeSecret as _;

use crate::data_cloud_dispatch::oauth::OauthM2m;
use crate::databricks_auth::oauth::{
    oauth_token_request, parse_oauth_token_reply, request_oauth_token,
};

use super::support::{Scripted, ok, status};

#[tokio::test]
async fn oauth_m2m_posts_the_form_and_reads_the_token() {
    let transport = Scripted::with_replies(vec![ok(r#"{"access_token":"tok"}"#)]);
    let request = oauth_token_request(
        "https://dbc.example.com/serving-endpoints",
        "client-id",
        "client-secret",
    );
    let token = request_oauth_token(
        &request,
        &OauthM2m {
            transport: &transport,
        },
    )
    .await
    .expect("the token reply is read");
    assert_eq!(token.expose_secret(), "tok");
    let sent = transport.sent();
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, "https://dbc.example.com/oidc/v1/token");
    assert_eq!(
        sent[0]
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        sent[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ=")
    );
    assert_eq!(
        sent[0].body.as_slice(),
        b"grant_type=client_credentials&scope=all-apis"
    );
    assert_eq!(sent[0].deadline, Duration::from_secs(30));
}

#[tokio::test]
async fn oauth_m2m_maps_an_error_status() {
    let transport = Scripted::with_replies(vec![status(401, "denied")]);
    let request = oauth_token_request("https://dbc.example.com", "id", "secret");
    match request_oauth_token(
        &request,
        &OauthM2m {
            transport: &transport,
        },
    )
    .await
    {
        Err(error) => {
            assert_eq!(error.status_code, 401);
            assert_eq!(error.message, "OAuth M2M token request failed: denied");
        }
        Ok(_) => panic!("expected the OAuth failure"),
    }
}

#[tokio::test]
async fn oauth_m2m_maps_a_transport_failure() {
    let transport = Scripted::default();
    let request = oauth_token_request("https://dbc.example.com", "id", "secret");
    match request_oauth_token(
        &request,
        &OauthM2m {
            transport: &transport,
        },
    )
    .await
    {
        Err(error) => {
            assert_eq!(error.status_code, 500);
            assert!(error.message.starts_with(
                "OAuth M2M token request failed: the provider answered with status 599"
            ));
        }
        Ok(_) => panic!("expected the OAuth failure"),
    }
}

#[test]
fn oauth_reply_decoding_keeps_the_status_and_the_body_text() {
    let error = parse_oauth_token_reply(500, "server error").expect_err("the status fails");
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "OAuth M2M token request failed: server error"
    );
    let token =
        parse_oauth_token_reply(200, r#"{"access_token":"abc"}"#).expect("the token reply is read");
    assert_eq!(token.expose_secret(), "abc");
}
