use http::StatusCode;
use secrecy::SecretString;

use crate::gigachat_tokens::authenticator::GigachatAuthenticator;
use crate::gigachat_tokens::error::GigachatAuthError;
use crate::gigachat_tokens::request::GigachatTokenRequest;
use crate::gigachat_tokens::runtime::SystemRuntime;
use crate::gigachat_tokens::tests::support;
use crate::gigachat_tokens::tokens::GigachatTokens as _;

const TOKEN_REPLY: &str = "{\"tok\":\"fetched-token\",\"exp\":4102444800000}";

fn request(auth_url: &str) -> GigachatTokenRequest {
    GigachatTokenRequest {
        access_token: None,
        credentials: None,
        scope: None,
        auth_url: Some(auth_url.to_owned()),
    }
}

#[tokio::test]
async fn source_order_returns_the_request_token_before_the_environment_token() {
    let server = support::Server::start().await;
    let settings = support::settings(&[
        ("GIGACHAT_ACCESS_TOKEN", "environment-token"),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let mut supplied = request(&server.url);
    supplied.access_token = Some(SecretString::from("request-token"));

    let token = auth
        .access_token(&supplied)
        .await
        .expect("the request token is returned");

    assert_eq!(support::text(&token), "request-token");
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn source_order_returns_the_environment_token_without_credentials() {
    let server = support::Server::start().await;
    let settings = support::settings(&[
        ("GIGACHAT_ACCESS_TOKEN", "environment-token"),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);

    let token = auth
        .access_token(&request(&server.url))
        .await
        .expect("the environment token needs no credentials");

    assert_eq!(support::text(&token), "environment-token");
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn source_order_skips_an_empty_request_token() {
    let server = support::Server::start().await;
    let settings = support::settings(&[
        ("GIGACHAT_ACCESS_TOKEN", "environment-token"),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let mut supplied = request(&server.url);
    supplied.access_token = Some(SecretString::from(""));

    let token = auth
        .access_token(&supplied)
        .await
        .expect("an empty request token falls back to the environment");

    assert_eq!(support::text(&token), "environment-token");
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn source_order_refuses_a_missing_token_and_missing_credentials() {
    let server = support::Server::start().await;
    let settings = support::settings(&[]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);

    let error = auth
        .access_token(&request(&server.url))
        .await
        .expect_err("a missing token and credentials are refused");

    assert!(matches!(&error, GigachatAuthError::MissingCredentials));
    assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn source_order_treats_empty_request_credentials_as_missing() {
    let server = support::Server::start().await;
    let settings = support::settings(&[]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let mut supplied = request(&server.url);
    supplied.credentials = Some(SecretString::from(""));

    let error = auth
        .access_token(&supplied)
        .await
        .expect_err("empty credentials are missing credentials");

    assert!(matches!(&error, GigachatAuthError::MissingCredentials));
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn source_order_takes_the_request_credentials_before_the_settings() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", "settings-credential"),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let mut supplied = request(&server.url);
    supplied.credentials = Some(SecretString::from("request-credential"));

    let token = auth
        .access_token(&supplied)
        .await
        .expect("the token is fetched with the request credentials");

    assert_eq!(support::text(&token), "fetched-token");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].header("authorization"),
        "Basic request-credential"
    );
}

#[tokio::test]
async fn source_order_skips_empty_request_credentials() {
    let server = support::Server::start().await;
    server.answer(200, TOKEN_REPLY);
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", "settings-credential"),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(settings, support::client(), SystemRuntime);
    let mut supplied = request(&server.url);
    supplied.credentials = Some(SecretString::from(""));

    let token = auth
        .access_token(&supplied)
        .await
        .expect("empty request credentials fall back to the settings");

    assert_eq!(support::text(&token), "fetched-token");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].header("authorization"),
        "Basic settings-credential"
    );
}
