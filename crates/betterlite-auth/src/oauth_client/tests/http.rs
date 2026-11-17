use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_models::controls::json_value::JsonValue;
use http::HeaderMap;
use http::header::{AUTHORIZATION, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::oauth_client::OAuthClient;
use crate::oauth_client::discovery::{DiscoverySource, fetch_discovery};
use crate::oauth_client::document::OAuthDiscoveryDocument;
use crate::oauth_client::error::OAuthClientError;
use crate::oauth_client::exchange::{CallbackRequest, ExchangeRequest, exchange};
use crate::oauth_client::tests::provider::{
    Provider, SeenRequest, serve, serve_sequence, transport,
};

fn client() -> OAuthClient {
    OAuthClient {
        client_id: "client-id".to_owned(),
        client_secret: SecretString::from("client-secret"),
        redirect_uri: Some("https://proxy.test/sso/callback".to_owned()),
        scopes: vec![
            "openid".to_owned(),
            "email".to_owned(),
            "profile".to_owned(),
        ],
        allow_insecure_http: true,
        additional_headers: HeaderMap::new(),
    }
}

fn callback(url: &str) -> CallbackRequest<'_> {
    CallbackRequest {
        url,
        sso_state_cookie: None,
    }
}

fn exchange_request<'a>(url: &'a str, include_client_id: bool) -> ExchangeRequest<'a> {
    ExchangeRequest {
        callback: callback(url),
        redirect_uri: None,
        include_client_id,
    }
}

fn discovery_source(provider: &Provider) -> DiscoverySource {
    DiscoverySource::WellKnown(format!("{}/discovery", provider.base))
}

fn form_value(body: &str, name: &str) -> String {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

fn discovery_body(request: &SeenRequest) -> (u16, String) {
    let host = request.header("host").unwrap();
    (
        200,
        format!(
            r#"{{"authorization_endpoint":"http://{host}/authorize","token_endpoint":"http://{host}/token","userinfo_endpoint":"http://{host}/userinfo"}}"#
        ),
    )
}

fn google_provider(request: &SeenRequest) -> (u16, String) {
    match request.target.split('?').next().unwrap() {
        "/discovery" => discovery_body(request),
        "/token" => {
            let code = form_value(&request.body, "code");
            (
                200,
                format!(
                    r#"{{"access_token":"access-{code}","refresh_token":"refresh-{code}","id_token":"id-{code}","token_type":"Bearer","expires_in":3600}}"#
                ),
            )
        }
        "/userinfo" => {
            let token = request
                .header("authorization")
                .and_then(|header| header.strip_prefix("Bearer "))
                .unwrap_or("unknown");
            (
                200,
                format!(
                    r#"{{"sub":"subject-{token}","email":"{token}@example.test","email_verified":true}}"#
                ),
            )
        }
        _ => (404, "{}".to_owned()),
    }
}

async fn provider() -> Provider {
    serve(Arc::new(google_provider)).await
}

async fn discovery(client: &OAuthClient, provider: &Provider) -> OAuthDiscoveryDocument {
    fetch_discovery(client, &transport(), &discovery_source(provider))
        .await
        .unwrap()
}

#[tokio::test]
async fn exchanges_a_callback_for_tokens_and_userinfo() {
    let provider = provider().await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    let exchanged = exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request(
            "https://proxy.test/sso/callback?code=abc&state=state-1",
            true,
        ),
    )
    .await
    .unwrap();
    assert_eq!(exchanged.tokens.access_token.expose_secret(), "access-abc");
    assert_eq!(
        exchanged
            .tokens
            .refresh_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("refresh-abc")
    );
    assert_eq!(
        exchanged
            .tokens
            .id_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("id-abc")
    );
    assert_eq!(
        exchanged.userinfo.fields.get("email"),
        Some(&JsonValue::Text("access-abc@example.test".to_owned()))
    );
    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].target, "/discovery");
    assert_eq!(seen[0].header("authorization"), None);
    assert_eq!(seen[1].method, "POST");
    assert_eq!(seen[1].target, "/token");
    let basic = format!("Basic {}", STANDARD.encode("client-id:client-secret"));
    assert_eq!(seen[1].header("authorization"), Some(basic.as_str()));
    assert_eq!(
        seen[1].header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(
        seen[1].body,
        "grant_type=authorization_code&client_id=client-id&code=abc&redirect_uri=https%3A%2F%2Fproxy.test%2Fsso%2Fcallback"
    );
    assert_eq!(seen[2].method, "GET");
    assert_eq!(seen[2].target, "/userinfo");
    assert_eq!(seen[2].header("authorization"), Some("Bearer access-abc"));
    assert_eq!(seen[2].body, "");
}

#[tokio::test]
async fn include_client_id_off_omits_the_body_client_id() {
    let provider = provider().await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", false),
    )
    .await
    .unwrap();
    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(
        seen[1].body,
        "grant_type=authorization_code&code=abc&redirect_uri=https%3A%2F%2Fproxy.test%2Fsso%2Fcallback"
    );
}

#[tokio::test]
async fn additional_headers_overlay_the_generated_headers() {
    let provider = provider().await;
    let mut client = client();
    client.additional_headers.insert(
        HeaderName::from_static("x-provider-extra"),
        HeaderValue::from_static("extra"),
    );
    client.additional_headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Custom credentials"),
    );
    let discovery = discovery(&client, &provider).await;
    exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", true),
    )
    .await
    .unwrap();
    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(seen[1].header("authorization"), Some("Custom credentials"));
    assert_eq!(seen[1].header("x-provider-extra"), Some("extra"));
    assert_eq!(seen[2].header("authorization"), Some("Custom credentials"));
    assert_eq!(seen[2].header("x-provider-extra"), Some("extra"));
}

#[tokio::test]
async fn a_rejected_token_answer_is_an_auth_failure() {
    let provider = serve(Arc::new(|request: &SeenRequest| {
        if request.target == "/discovery" {
            return discovery_body(request);
        }
        (400, r#"{"error":"invalid_grant"}"#.to_owned())
    }))
    .await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    let error = exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", true),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(&error, OAuthClientError::Token(_)));
    assert_eq!(error.response().status, 401);
}

#[tokio::test]
async fn a_rejected_userinfo_answer_is_an_auth_failure() {
    let provider = serve(Arc::new(|request: &SeenRequest| {
        if request.target == "/discovery" {
            return discovery_body(request);
        }
        if request.target == "/userinfo" {
            return (500, "{}".to_owned());
        }
        (200, r#"{"access_token":"at"}"#.to_owned())
    }))
    .await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    let error = exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", true),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(&error, OAuthClientError::Userinfo(_)));
    assert_eq!(error.response().status, 401);
}

#[tokio::test]
async fn a_malformed_token_document_is_an_auth_failure() {
    let provider = serve(Arc::new(|request: &SeenRequest| {
        if request.target == "/discovery" {
            return discovery_body(request);
        }
        (200, "not json".to_owned())
    }))
    .await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    let error = exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", true),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(&error, OAuthClientError::Token(_)));
    assert_eq!(error.response().status, 401);
}

#[tokio::test]
async fn a_token_response_without_an_access_token_is_an_auth_failure() {
    let provider = serve(Arc::new(|request: &SeenRequest| {
        if request.target == "/discovery" {
            return discovery_body(request);
        }
        (200, r#"{"access_token":""}"#.to_owned())
    }))
    .await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    let error = exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", true),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(&error, OAuthClientError::Token(_)));
    assert_eq!(error.response().status, 401);
}

#[tokio::test]
async fn an_access_token_a_header_cannot_carry_is_a_server_error() {
    let provider = serve(Arc::new(|request: &SeenRequest| {
        if request.target == "/discovery" {
            return discovery_body(request);
        }
        (200, r#"{"access_token":"bad\ntoken"}"#.to_owned())
    }))
    .await;
    let client = client();
    let discovery = discovery(&client, &provider).await;
    let error = exchange(
        &client,
        &transport(),
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=abc", true),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(&error, OAuthClientError::Credentials(_)));
    assert_eq!(error.response().status, 500);
}

#[tokio::test]
async fn a_discovery_answer_that_is_not_the_document_is_a_provider_failure() {
    let provider = serve(Arc::new(|_request: &SeenRequest| {
        (404, r#"{"error":"not found"}"#.to_owned())
    }))
    .await;
    let error = fetch_discovery(&client(), &transport(), &discovery_source(&provider))
        .await
        .err()
        .unwrap();
    assert!(matches!(&error, OAuthClientError::Discovery(_)));
    assert_eq!(error.response().status, 503);
    assert_eq!(
        error.response().error_type,
        ProxyErrorTypes::AuthProviderUnavailable
    );
}

#[tokio::test]
async fn an_inline_discovery_document_needs_no_request() {
    let provider = serve_sequence(Vec::new()).await;
    let document = OAuthDiscoveryDocument {
        authorization_endpoint: "https://provider.test/authorize".to_owned(),
        token_endpoint: "https://provider.test/token".to_owned(),
        userinfo_endpoint: "https://provider.test/userinfo".to_owned(),
    };
    let fetched = fetch_discovery(
        &client(),
        &transport(),
        &DiscoverySource::Inline(document.clone()),
    )
    .await
    .unwrap();
    assert_eq!(fetched, document);
    assert!(provider.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_insecure_discovery_url_needs_the_opt_in() {
    let provider = serve_sequence(Vec::new()).await;
    let mut client = client();
    client.allow_insecure_http = false;
    let error = fetch_discovery(&client, &transport(), &discovery_source(&provider))
        .await
        .err()
        .unwrap();
    assert!(matches!(&error, OAuthClientError::Insecure { .. }));
    assert_eq!(error.response().status, 500);
    assert!(provider.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn overlapping_logins_keep_their_own_tokens() {
    let provider = provider().await;
    let client = client();
    let transport = transport();
    let discovery = discovery(&client, &provider).await;
    let alpha = exchange(
        &client,
        &transport,
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=alpha", true),
    );
    let beta = exchange(
        &client,
        &transport,
        &discovery,
        &exchange_request("https://proxy.test/sso/callback?code=beta", true),
    );
    let (alpha, beta) = tokio::join!(alpha, beta);
    let alpha = alpha.unwrap();
    let beta = beta.unwrap();
    assert_eq!(alpha.tokens.access_token.expose_secret(), "access-alpha");
    assert_eq!(beta.tokens.access_token.expose_secret(), "access-beta");
    assert_eq!(
        alpha
            .tokens
            .refresh_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("refresh-alpha")
    );
    assert_eq!(
        beta.tokens
            .id_token
            .as_ref()
            .map(|token| token.expose_secret()),
        Some("id-beta")
    );
    assert_eq!(
        alpha.userinfo.fields.get("email"),
        Some(&JsonValue::Text("access-alpha@example.test".to_owned()))
    );
    assert_eq!(
        beta.userinfo.fields.get("email"),
        Some(&JsonValue::Text("access-beta@example.test".to_owned()))
    );
}
