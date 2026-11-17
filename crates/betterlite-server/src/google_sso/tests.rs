#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_auth::oauth_client::error::OAuthClientError;
use betterlite_auth::oauth_client::google::GOOGLE_DISCOVERY_URL;
use betterlite_config::google_sso::GOOGLE_CLIENT_SECRET_ENV;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::{
    GoogleCallback, GoogleSsoConfig, GoogleSsoError, GoogleSsoResponse,
    get_google_callback_response,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveryReply {
    Document,
    NotFound,
}

#[derive(Debug, Clone)]
struct Seen {
    method: &'static str,
    path: String,
    authorization: Option<String>,
    body: String,
}

#[derive(Clone)]
struct Provider {
    base: String,
    email_verified: bool,
    discovery: DiscoveryReply,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Provider {
    fn record(&self, method: &'static str, uri_path: &str, headers: &HeaderMap, body: &str) {
        self.seen.lock().unwrap().push(Seen {
            method,
            path: uri_path.to_owned(),
            authorization: headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            body: body.to_owned(),
        });
    }
}

struct Running {
    provider: Provider,
    tasks: JoinSet<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.tasks.abort_all();
    }
}

async fn read_answer(
    State(provider): State<Provider>,
    uri: Uri,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    provider.record("GET", uri.path(), &headers, "");
    if uri.path() == "/discovery" {
        return match provider.discovery {
            DiscoveryReply::Document => (
                StatusCode::OK,
                Json(json!({
                    "authorization_endpoint": format!("{}/authorize", provider.base),
                    "token_endpoint": format!("{}/token", provider.base),
                    "userinfo_endpoint": format!("{}/userinfo", provider.base),
                })),
            ),
            DiscoveryReply::NotFound => {
                (StatusCode::NOT_FOUND, Json(json!({"error": "not found"})))
            }
        };
    }
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("unknown");
    (
        StatusCode::OK,
        Json(json!({
            "sub": format!("subject-{token}"),
            "email": format!("{token}@example.test"),
            "email_verified": provider.email_verified,
            "given_name": "Ann",
            "family_name": "Lee",
            "name": "Ann Lee",
            "picture": "https://img.example.test/ann.png",
        })),
    )
}

async fn token_answer(
    State(provider): State<Provider>,
    headers: HeaderMap,
    body: String,
) -> Json<Value> {
    provider.record("POST", "/token", &headers, &body);
    let code = form_value(&body, "code");
    Json(json!({
        "access_token": format!("access-{code}"),
        "refresh_token": format!("refresh-{code}"),
        "id_token": format!("id-{code}"),
        "token_type": "Bearer",
    }))
}

fn form_value(body: &str, name: &str) -> String {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}

async fn serve(email_verified: bool, reply: DiscoveryReply) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let provider = Provider {
        base,
        email_verified,
        discovery: reply,
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let router = Router::new()
        .route("/discovery", get(read_answer))
        .route("/token", post(token_answer))
        .route("/userinfo", get(read_answer))
        .with_state(provider.clone());
    let mut tasks = JoinSet::new();
    tasks.spawn(async move { axum::serve(listener, router).await.unwrap() });
    Running { provider, tasks }
}

fn transport() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

fn config(provider: &Provider) -> GoogleSsoConfig {
    GoogleSsoConfig {
        client_id: "client-id".to_owned(),
        redirect_url: "https://proxy.test/sso/callback".to_owned(),
        client_secret: Some(SecretString::from("client-secret")),
        discovery_url: Some(format!("{}/discovery", provider.base)),
        allow_insecure_http: true,
    }
}

fn callback<'a>(url: &'a str, sso_state_cookie: Option<&'a str>) -> GoogleCallback<'a> {
    GoogleCallback {
        url,
        sso_state_cookie,
    }
}

#[test]
fn the_discovery_url_defaults_to_google() {
    let mut config = GoogleSsoConfig {
        client_id: "client-id".to_owned(),
        redirect_url: "https://proxy.test/sso/callback".to_owned(),
        client_secret: None,
        discovery_url: None,
        allow_insecure_http: false,
    };
    assert_eq!(config.discovery_url(), GOOGLE_DISCOVERY_URL);
    config.discovery_url = Some("https://provider.test/discovery".to_owned());
    assert_eq!(config.discovery_url(), "https://provider.test/discovery");
}

#[tokio::test]
async fn a_missing_client_secret_is_a_server_error() {
    let running = serve(true, DiscoveryReply::Document).await;
    let mut config = config(&running.provider);
    config.client_secret = None;
    let error = get_google_callback_response(
        &transport(),
        &config,
        &callback("https://proxy.test/sso/callback?code=abc", None),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, GoogleSsoError::MissingClientSecret));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type),
        (500, ProxyErrorTypes::AuthError)
    );
    assert_eq!(response.param, Some(GOOGLE_CLIENT_SECRET_ENV));
    assert_eq!(response.code, "500");
    assert_eq!(
        error.to_string(),
        "GOOGLE_CLIENT_SECRET not set. Set it in .env file"
    );
    assert!(running.provider.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_callback_exchanges_the_code_and_converts_the_identity() {
    let running = serve(true, DiscoveryReply::Document).await;
    let response = get_google_callback_response(
        &transport(),
        &config(&running.provider),
        &callback(
            "https://proxy.test/sso/callback?code=abc&state=state-1",
            Some("state-1"),
        ),
        false,
    )
    .await
    .unwrap();
    let GoogleSsoResponse::Converted(identity) = response else {
        panic!("the callback converts the identity");
    };
    assert_eq!(
        serde_json::to_value(identity).unwrap(),
        json!({
            "id": "subject-access-abc",
            "email": "access-abc@example.test",
            "first_name": "Ann",
            "last_name": "Lee",
            "display_name": "Ann Lee",
            "picture": "https://img.example.test/ann.png",
            "provider": "google",
            "team_ids": [],
            "user_role": null,
            "extra_fields": null,
        })
    );
    let seen = running.provider.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    assert_eq!(
        (seen[0].method, seen[0].path.as_str()),
        ("GET", "/discovery")
    );
    assert_eq!((seen[1].method, seen[1].path.as_str()), ("POST", "/token"));
    let basic = format!("Basic {}", STANDARD.encode("client-id:client-secret"));
    assert_eq!(seen[1].authorization.as_deref(), Some(basic.as_str()));
    assert_eq!(form_value(&seen[1].body, "client_id"), "client-id");
    assert_eq!(form_value(&seen[1].body, "code"), "abc");
    assert_eq!(
        form_value(&seen[1].body, "redirect_uri"),
        "https://proxy.test/sso/callback"
    );
    assert_eq!(
        (seen[2].method, seen[2].path.as_str()),
        ("GET", "/userinfo")
    );
    assert_eq!(seen[2].authorization.as_deref(), Some("Bearer access-abc"));
}

#[tokio::test]
async fn a_raw_response_bypasses_the_conversion() {
    let running = serve(false, DiscoveryReply::Document).await;
    let response = get_google_callback_response(
        &transport(),
        &config(&running.provider),
        &callback("https://proxy.test/sso/callback?code=abc", None),
        true,
    )
    .await
    .unwrap();
    let GoogleSsoResponse::Raw(userinfo) = response else {
        panic!("raw mode returns the userinfo document");
    };
    assert_eq!(
        userinfo.fields.get("email"),
        Some(&JsonValue::Text("access-abc@example.test".to_owned()))
    );
    assert_eq!(
        userinfo.fields.get("email_verified"),
        Some(&JsonValue::Bool(false))
    );
}

#[tokio::test]
async fn an_unverified_google_user_is_refused() {
    let running = serve(false, DiscoveryReply::Document).await;
    let error = get_google_callback_response(
        &transport(),
        &config(&running.provider),
        &callback("https://proxy.test/sso/callback?code=abc", None),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(&error, GoogleSsoError::Identity(_)));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type),
        (401, ProxyErrorTypes::AuthError)
    );
    assert_eq!(
        error.to_string(),
        "User access-abc@example.test is not verified with Google"
    );
}

#[tokio::test]
async fn a_state_cookie_that_differs_from_the_state_is_refused() {
    let running = serve(true, DiscoveryReply::Document).await;
    let error = get_google_callback_response(
        &transport(),
        &config(&running.provider),
        &callback(
            "https://proxy.test/sso/callback?code=abc&state=one",
            Some("two"),
        ),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        GoogleSsoError::Client(OAuthClientError::InvalidState)
    ));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type, response.param),
        (401, ProxyErrorTypes::AuthError, Some("state"))
    );
    assert!(running.provider.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_callback_without_a_code_is_a_bad_request() {
    let running = serve(true, DiscoveryReply::Document).await;
    let error = get_google_callback_response(
        &transport(),
        &config(&running.provider),
        &callback("https://proxy.test/sso/callback?state=one", None),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        GoogleSsoError::Client(OAuthClientError::MissingCode)
    ));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type, response.param),
        (400, ProxyErrorTypes::AuthError, Some("code"))
    );
    assert!(running.provider.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_discovery_answer_that_is_not_the_document_is_a_provider_failure() {
    let running = serve(true, DiscoveryReply::NotFound).await;
    let error = get_google_callback_response(
        &transport(),
        &config(&running.provider),
        &callback("https://proxy.test/sso/callback?code=abc", None),
        false,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        GoogleSsoError::Client(OAuthClientError::Discovery(_))
    ));
    let response = error.response();
    assert_eq!(
        (response.status, response.error_type),
        (503, ProxyErrorTypes::AuthProviderUnavailable)
    );
}
