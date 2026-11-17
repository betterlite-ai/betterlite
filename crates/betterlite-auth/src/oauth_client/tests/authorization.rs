use http::HeaderMap;
use secrecy::SecretString;

use crate::oauth_client::OAuthClient;
use crate::oauth_client::authorization::{AuthorizationRequest, authorization_url};
use crate::oauth_client::error::OAuthClientError;

fn client(scopes: &[&str], redirect_uri: Option<&str>, allow_insecure_http: bool) -> OAuthClient {
    OAuthClient {
        client_id: "client-id".to_owned(),
        client_secret: SecretString::from("client-secret"),
        redirect_uri: redirect_uri.map(str::to_owned),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        allow_insecure_http,
        additional_headers: HeaderMap::new(),
    }
}

#[test]
fn builds_the_authorization_url_in_the_oauth_lib_order() {
    let client = client(
        &["openid", "email", "profile"],
        Some("http://localhost/cb"),
        true,
    );
    let request = AuthorizationRequest {
        endpoint: "https://accounts.example.test/o/oauth2/v2/auth".to_owned(),
        state: Some("state-1".to_owned()),
        redirect_uri: None,
        extra_params: vec![
            ("code_challenge".to_owned(), "challenge".to_owned()),
            ("empty".to_owned(), String::new()),
        ],
    };
    let url = authorization_url(&client, &request).unwrap();
    assert_eq!(
        url.as_str(),
        "https://accounts.example.test/o/oauth2/v2/auth?response_type=code&client_id=client-id&redirect_uri=http%3A%2F%2Flocalhost%2Fcb&scope=openid+email+profile&state=state-1&code_challenge=challenge"
    );
}

#[test]
fn keeps_the_existing_query_and_omits_empty_values() {
    let client = client(&[], None, true);
    let request = AuthorizationRequest {
        endpoint: "https://accounts.example.test/authorize?prompt=select_account".to_owned(),
        state: Some(String::new()),
        redirect_uri: Some("https://proxy.example.test/sso/callback".to_owned()),
        extra_params: Vec::new(),
    };
    let url = authorization_url(&client, &request).unwrap();
    assert_eq!(
        url.as_str(),
        "https://accounts.example.test/authorize?prompt=select_account&response_type=code&client_id=client-id&redirect_uri=https%3A%2F%2Fproxy.example.test%2Fsso%2Fcallback"
    );
}

#[test]
fn the_request_redirect_uri_wins_over_the_client_one() {
    let client = client(&[], Some("https://client.test/cb"), true);
    let request = AuthorizationRequest {
        endpoint: "https://provider.test/authorize".to_owned(),
        state: None,
        redirect_uri: Some("https://request.test/cb".to_owned()),
        extra_params: Vec::new(),
    };
    let url = authorization_url(&client, &request).unwrap();
    let redirect_uri = url
        .query_pairs()
        .find(|(name, _)| name == "redirect_uri")
        .map(|(_, value)| value.into_owned());
    assert_eq!(redirect_uri, Some("https://request.test/cb".to_owned()));
}

#[test]
fn a_request_without_a_redirect_uri_is_refused() {
    let client = client(&[], None, true);
    let request = AuthorizationRequest {
        endpoint: "https://provider.test/authorize".to_owned(),
        state: None,
        redirect_uri: None,
        extra_params: Vec::new(),
    };
    let error = authorization_url(&client, &request).unwrap_err();
    assert!(matches!(&error, OAuthClientError::MissingRedirectUri));
    assert_eq!(error.response().status, 500);
}

#[test]
fn an_insecure_authorization_endpoint_needs_the_opt_in() {
    let request = AuthorizationRequest {
        endpoint: "http://provider.test/authorize".to_owned(),
        state: None,
        redirect_uri: None,
        extra_params: Vec::new(),
    };
    let refused = authorization_url(&client(&[], Some("https://proxy.test/cb"), false), &request)
        .unwrap_err();
    assert!(matches!(&refused, OAuthClientError::Insecure { .. }));
    assert_eq!(refused.response().status, 500);
    let allowed = authorization_url(&client(&[], Some("https://proxy.test/cb"), true), &request);
    assert!(
        allowed
            .unwrap()
            .as_str()
            .starts_with("http://provider.test/authorize?")
    );
}

#[test]
fn an_authorization_endpoint_that_is_not_a_url_is_a_provider_failure() {
    let client = client(&[], Some("https://proxy.test/cb"), true);
    let request = AuthorizationRequest {
        endpoint: String::new(),
        state: None,
        redirect_uri: None,
        extra_params: Vec::new(),
    };
    let error = authorization_url(&client, &request).unwrap_err();
    assert!(matches!(&error, OAuthClientError::Endpoint { .. }));
    assert_eq!(error.response().status, 503);
}
