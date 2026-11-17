use std::time::Duration;

use betterlite_error::proxy_error_types::ProxyErrorTypes;
use rstest::rstest;
use secrecy::SecretString;
use tokio::net::TcpListener;

use crate::oidc_token_exchange::decode::ProviderClaims;
use crate::oidc_token_exchange::exchange::{
    OidcTokenExchangeError, TOKEN_EXCHANGE_PARAM, TOKEN_EXCHANGE_TIMEOUT, TokenExchangeRequest,
    USERINFO_PARAM, USERINFO_TIMEOUT, exchange_pkce_code,
};

use super::support::{
    LocalProvider, Reply, VALID_ID_TOKEN, base_request, client, form, text, token_reply,
};

async fn run_exchange(
    provider: &LocalProvider,
    request: &TokenExchangeRequest,
) -> Result<ProviderClaims, OidcTokenExchangeError> {
    exchange_pkce_code(&client(), request).await
}

#[tokio::test]
async fn sends_the_pinned_form_body_and_headers_with_basic_auth() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    provider.reply("/userinfo", Reply::json(r#"{"sub":"user-1"}"#));
    let claims = run_exchange(&provider, &base_request(&provider))
        .await
        .unwrap();
    let token = &provider.requests()[0];
    assert_eq!(token.method, "POST");
    assert_eq!(token.path, "/token");
    assert_eq!(
        token.header("content-type"),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(token.header("accept"), "application/json");
    assert_eq!(
        token.header("authorization"),
        "Basic Y2xpZW50LTE6Y2xpZW50LXNlY3JldA=="
    );
    let fields = form(&token.body);
    assert_eq!(
        fields.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(fields.get("code").map(String::as_str), Some("code-1"));
    assert_eq!(
        fields.get("code_verifier").map(String::as_str),
        Some("verifier-1")
    );
    assert_eq!(
        fields.get("redirect_uri").map(String::as_str),
        Some("https://proxy.example/sso/callback")
    );
    assert!(!fields.contains_key("client_id"));
    assert!(!fields.contains_key("client_secret"));
    assert_eq!(text(&claims, "access_token"), Some("access-1"));
}

#[tokio::test]
async fn fetches_userinfo_with_the_issued_bearer_token() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    provider.reply("/userinfo", Reply::json(r#"{"sub":"user-1"}"#));
    let claims = run_exchange(&provider, &base_request(&provider))
        .await
        .unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let userinfo = &requests[1];
    assert_eq!(userinfo.method, "GET");
    assert_eq!(userinfo.path, "/userinfo");
    assert_eq!(userinfo.header("authorization"), "Bearer access-1");
    assert_eq!(text(&claims, "sub"), Some("user-1"));
    assert_eq!(text(&claims, "access_token"), Some("access-1"));
}

#[tokio::test]
async fn puts_client_id_in_the_body_for_a_public_client() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.client_secret = None;
    request.userinfo_endpoint = None;
    run_exchange(&provider, &request).await.unwrap();
    let token = &provider.requests()[0];
    assert_eq!(token.header("authorization"), "");
    let fields = form(&token.body);
    assert_eq!(
        fields.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert!(!fields.contains_key("client_secret"));
}

#[tokio::test]
async fn treats_an_empty_client_secret_as_absent() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.client_secret = Some(SecretString::from(String::new()));
    request.userinfo_endpoint = None;
    run_exchange(&provider, &request).await.unwrap();
    let token = &provider.requests()[0];
    assert_eq!(token.header("authorization"), "");
    assert_eq!(
        form(&token.body).get("client_id").map(String::as_str),
        Some("client-1")
    );
}

#[tokio::test]
async fn puts_client_credentials_in_the_body_when_include_client_id_is_set() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.include_client_id = true;
    request.userinfo_endpoint = None;
    run_exchange(&provider, &request).await.unwrap();
    let token = &provider.requests()[0];
    assert_eq!(token.header("authorization"), "");
    let fields = form(&token.body);
    assert_eq!(
        fields.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert_eq!(
        fields.get("client_secret").map(String::as_str),
        Some("client-secret")
    );
}

#[tokio::test]
async fn include_client_id_without_a_secret_sends_only_the_client_id() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.include_client_id = true;
    request.client_secret = None;
    request.userinfo_endpoint = None;
    run_exchange(&provider, &request).await.unwrap();
    let fields = form(&provider.requests()[0].body);
    assert_eq!(
        fields.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert!(!fields.contains_key("client_secret"));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn omits_an_absent_or_empty_redirect_uri(#[case] redirect_url: Option<&str>) {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.redirect_url = redirect_url.map(str::to_owned);
    request.userinfo_endpoint = None;
    run_exchange(&provider, &request).await.unwrap();
    assert!(!form(&provider.requests()[0].body).contains_key("redirect_uri"));
}

#[tokio::test]
async fn encodes_the_form_like_urllib() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.authorization_code = "code 1~*&=+".to_owned();
    request.userinfo_endpoint = None;
    run_exchange(&provider, &request).await.unwrap();
    assert_eq!(
        provider.requests()[0].body,
        "grant_type=authorization_code&code=code+1~%2A%26%3D%2B&code_verifier=verifier-1&redirect_uri=https%3A%2F%2Fproxy.example%2Fsso%2Fcallback"
    );
}

#[tokio::test]
async fn pins_content_type_and_accept_over_additional_headers() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    provider.reply("/userinfo", Reply::json(r#"{"sub":"user-1"}"#));
    let mut request = base_request(&provider);
    request.additional_headers = vec![
        ("content-type".to_owned(), "text/plain".to_owned()),
        ("accept".to_owned(), "text/html".to_owned()),
        ("x-provider".to_owned(), "sso".to_owned()),
    ];
    run_exchange(&provider, &request).await.unwrap();
    let requests = provider.requests();
    assert_eq!(
        requests[0].header("content-type"),
        "application/x-www-form-urlencoded"
    );
    assert_eq!(requests[0].header("accept"), "application/json");
    assert_eq!(requests[1].header("accept"), "text/html");
    assert_eq!(requests[0].header("x-provider"), "sso");
    assert_eq!(requests[1].header("x-provider"), "sso");
}

#[tokio::test]
async fn userinfo_bearer_overrides_an_additional_authorization_header() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    provider.reply("/userinfo", Reply::json(r#"{"sub":"user-1"}"#));
    let mut request = base_request(&provider);
    request.additional_headers =
        vec![("Authorization".to_owned(), "Bearer stale-header".to_owned())];
    run_exchange(&provider, &request).await.unwrap();
    let requests = provider.requests();
    assert_eq!(requests[1].header("authorization"), "Bearer access-1");
    assert_eq!(
        requests[0].header("authorization"),
        "Basic Y2xpZW50LTE6Y2xpZW50LXNlY3JldA=="
    );
}

#[tokio::test]
async fn follows_a_redirected_token_endpoint() {
    let provider = LocalProvider::start().await;
    provider.reply(
        "/token",
        Reply::redirect(&provider.endpoint("/token-final")),
    );
    provider.reply("/token-final", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.userinfo_endpoint = None;
    let claims = run_exchange(&provider, &request).await.unwrap();
    assert_eq!(text(&claims, "access_token"), Some("access-1"));
    let paths = provider
        .requests()
        .iter()
        .map(|recorded| recorded.path.clone())
        .collect::<Vec<String>>();
    assert_eq!(paths, vec!["/token".to_owned(), "/token-final".to_owned()]);
}

#[test]
fn uses_the_pinned_deadlines() {
    assert_eq!(TOKEN_EXCHANGE_TIMEOUT, Duration::from_secs(30));
    assert_eq!(USERINFO_TIMEOUT, Duration::from_secs(600));
}

#[tokio::test]
async fn maps_a_refused_connection_to_token_exchange_401() {
    let provider = LocalProvider::start().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut request = base_request(&provider);
    request.token_endpoint = format!("http://{address}/token");
    let error = run_exchange(&provider, &request).await.unwrap_err();
    assert_token_failure(&error);
}

#[rstest]
#[case::status_400(400, r#"{"access_token":"hidden"}"#)]
#[case::status_500(500, "provider exploded")]
#[case::accepted_html(200, "<html>")]
#[case::accepted_list(200, "[1]")]
#[case::accepted_missing_access_token(200, r#"{"token_type":"Bearer"}"#)]
#[case::accepted_empty_object(200, "{}")]
#[tokio::test]
async fn maps_token_endpoint_failures_to_token_exchange_401(
    #[case] status: u16,
    #[case] body: &str,
) {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::status(status, body));
    let error = run_exchange(&provider, &base_request(&provider))
        .await
        .unwrap_err();
    assert_token_failure(&error);
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test]
async fn rejects_a_token_endpoint_url_that_does_not_parse() {
    let provider = LocalProvider::start().await;
    let mut request = base_request(&provider);
    request.token_endpoint = "not a url".to_owned();
    let error = run_exchange(&provider, &request).await.unwrap_err();
    assert_token_failure(&error);
}

#[rstest]
#[case::server_error(500, "provider exploded")]
#[case::invalid_json(200, "not-json")]
#[case::json_null(200, "null")]
#[case::empty_object(200, "{}")]
#[case::empty_list(200, "[]")]
#[case::empty_string(200, r#""""#)]
#[tokio::test]
async fn falls_back_to_the_id_token_when_userinfo_carries_no_claims(
    #[case] status: u16,
    #[case] body: &str,
) {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(Some(VALID_ID_TOKEN))));
    provider.reply("/userinfo", Reply::status(status, body));
    let claims = run_exchange(&provider, &base_request(&provider))
        .await
        .unwrap();
    assert_eq!(text(&claims, "sub"), Some("user-1"));
    assert_eq!(text(&claims, "email"), Some("u@example.com"));
    assert_eq!(text(&claims, "access_token"), Some("access-1"));
    assert_eq!(text(&claims, "id_token"), Some(VALID_ID_TOKEN));
}

#[tokio::test]
async fn falls_back_to_the_id_token_without_a_userinfo_endpoint() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(Some(VALID_ID_TOKEN))));
    let mut request = base_request(&provider);
    request.userinfo_endpoint = None;
    let claims = run_exchange(&provider, &request).await.unwrap();
    assert_eq!(text(&claims, "sub"), Some("user-1"));
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test]
async fn keeps_userinfo_claims_when_the_token_response_carries_no_id_token() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    provider.reply(
        "/userinfo",
        Reply::json(r#"{"sub":"user-1","roles":["admin"]}"#),
    );
    let claims = run_exchange(&provider, &base_request(&provider))
        .await
        .unwrap();
    assert_eq!(text(&claims, "sub"), Some("user-1"));
    assert_eq!(text(&claims, "access_token"), Some("access-1"));
    assert!(claims.contains_key("roles"));
}

#[rstest]
#[case::no_id_token(r#"{"access_token":"access-1"}"#)]
#[case::empty_id_token(r#"{"access_token":"access-1","id_token":""}"#)]
#[case::numeric_id_token(r#"{"access_token":"access-1","id_token":42}"#)]
#[case::null_id_token(r#"{"access_token":"access-1","id_token":null}"#)]
#[case::invalid_jwt(r#"{"access_token":"access-1","id_token":"not-a-jwt"}"#)]
#[case::empty_payload(r#"{"access_token":"access-1","id_token":"eyJhbGciOiJSUzI1NiJ9.e30."}"#)]
#[tokio::test]
async fn fails_with_userinfo_401_when_the_id_token_cannot_supply_claims(#[case] token_body: &str) {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(token_body));
    provider.reply("/userinfo", Reply::status(500, "provider exploded"));
    let error = run_exchange(&provider, &base_request(&provider))
        .await
        .unwrap_err();
    let response = error.response();
    assert_eq!(response.status, 401);
    assert_eq!(response.error_type, ProxyErrorTypes::AuthError);
    assert_eq!(response.param, Some(USERINFO_PARAM));
    assert_eq!(response.code, "401");
}

#[tokio::test]
async fn fails_with_userinfo_401_without_a_userinfo_endpoint_or_id_token() {
    let provider = LocalProvider::start().await;
    provider.reply("/token", Reply::json(&token_reply(None)));
    let mut request = base_request(&provider);
    request.userinfo_endpoint = None;
    let error = run_exchange(&provider, &request).await.unwrap_err();
    assert_eq!(error.response().param, Some(USERINFO_PARAM));
    assert_eq!(error.response().status, 401);
}

fn assert_token_failure(error: &OidcTokenExchangeError) {
    let response = error.response();
    assert_eq!(response.status, 401);
    assert_eq!(response.error_type, ProxyErrorTypes::AuthError);
    assert_eq!(response.param, Some(TOKEN_EXCHANGE_PARAM));
    assert_eq!(response.code, "401");
}
