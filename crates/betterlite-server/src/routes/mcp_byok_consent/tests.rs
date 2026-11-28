#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use betterlite_auth::auth_key_lookup::error::KeyLookupError;
use betterlite_auth::auth_key_lookup::{KeyAuthenticator, KeyLookupOutcome};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_mcp::byok_consent::store::{AUTH_CODES_MAX_SIZE, AuthCodeStore};
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup as _, AuthCodeRecord};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use secrecy::SecretString;
use serde::Serialize;
use tower::ServiceExt as _;

use super::{AUTHORIZE_PATH, ConsentState, router_with};
use crate::api::request_form::MAX_FORM_FIELDS;
use crate::routes::router;
use crate::routes::tests::{empty_config, state_for};

const MASTER_KEY: &str = "sk-oracle-master";
const CALLBACK: &str = "http://127.0.0.1:1456/callback";
const FORM_TYPE: &str = "application/x-www-form-urlencoded";
const MULTIPART_FORM: &str = concat!(
    "--boundary-1\r\nContent-Disposition: form-data; name=\"redirect_uri\"\r\n\r\n",
    "http://127.0.0.1:1456/callback\r\n",
    "--boundary-1\r\nContent-Disposition: form-data; name=\"code_challenge\"\r\n\r\n",
    "challenge\r\n",
    "--boundary-1\r\nContent-Disposition: form-data; name=\"api_key\"\r\n\r\n",
    "sk-user\r\n",
    "--boundary-1--\r\n"
);
const FUTURE_EXP: i64 = 4_102_444_800;

#[derive(Serialize)]
struct CookieClaims<'text> {
    exp: i64,
    login_method: &'text str,
    user_id: &'text str,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<&'text str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    token_type: Option<&'text str>,
}

fn ui_claims() -> CookieClaims<'static> {
    CookieClaims {
        exp: FUTURE_EXP,
        login_method: "sso",
        user_id: "user-1",
        key: None,
        token_type: None,
    }
}

fn cookie(claims: &CookieClaims<'_>, master_key: &str) -> String {
    let token = encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(master_key.as_bytes()),
    )
    .unwrap();
    format!("token={token}")
}

struct Keys {
    accepts: bool,
}

#[async_trait]
impl KeyAuthenticator for Keys {
    async fn authenticate(&self, presented: &str) -> Result<KeyLookupOutcome, KeyLookupError> {
        if !self.accepts {
            return Err(KeyLookupError::MissingKey {
                key: presented.to_owned(),
            });
        }
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
        Ok(KeyLookupOutcome {
            auth,
            hashed_token: presented.to_owned(),
        })
    }
}

fn keyring(accepts: bool) -> Option<Arc<dyn KeyAuthenticator>> {
    let keys: Arc<dyn KeyAuthenticator> = Arc::new(Keys { accepts });
    Some(keys)
}

fn app(
    codes: AuthCodeStore,
    master_key: Option<&str>,
    authenticator: Option<Arc<dyn KeyAuthenticator>>,
) -> Router {
    router_with(Arc::new(ConsentState {
        codes,
        master_key: master_key.map(SecretString::from),
        authenticator,
    }))
}

fn signed_in(codes: AuthCodeStore) -> Router {
    app(codes, Some(MASTER_KEY), keyring(true))
}

fn authorize_uri(query: &str) -> String {
    format!("{AUTHORIZE_PATH}?{query}")
}

fn valid_query() -> String {
    format!(
        "response_type=code&client_id=client-1&redirect_uri={CALLBACK}&code_challenge=challenge&state=state-1"
    )
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post(uri: &str, body: &str, content_type: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, content_type);
    if let Some(cookie_value) = cookie {
        builder = builder.header(header::COOKIE, cookie_value);
    }
    builder.body(Body::from(body.to_owned())).unwrap()
}

async fn send(router: Router, request: Request<Body>) -> Response {
    router.oneshot(request).await.unwrap()
}

async fn text(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 0x0010_0000).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn header_of<'response>(response: &'response Response, name: &str) -> Option<&'response str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn valid_form() -> String {
    format!("redirect_uri={CALLBACK}&code_challenge=challenge&api_key=sk-user")
}

fn live_seed() -> AuthCodeRecord {
    AuthCodeRecord {
        api_key: SecretString::from("sk-seed"),
        server_id: String::new(),
        code_challenge: String::new(),
        redirect_uri: String::new(),
        client_id: String::new(),
        user_id: String::new(),
        expires_at: f64::MAX,
    }
}

#[tokio::test]
async fn the_get_page_needs_no_cookie_and_serves_html() {
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(&valid_query())),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_of(&response, "content-type"),
        Some("text/html; charset=utf-8")
    );
    let body = text(response).await;
    assert!(body.contains("id=\"authForm\""));
    assert!(body.contains("name=\"redirect_uri\" value=\"http://127.0.0.1:1456/callback\""));
}

#[tokio::test]
async fn the_get_falls_back_until_the_registry_is_wired_into_the_state() {
    let base = valid_query();
    let query = format!("{base}&server_id=srv-1");
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(&query)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(text(response).await.contains("Connect MCP Server MCP"));
}

#[tokio::test]
async fn the_get_refuses_a_missing_code_challenge() {
    let query = "response_type=code&redirect_uri=http://127.0.0.1:1456/callback";
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(query)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        header_of(&response, "content-type"),
        Some("application/json")
    );
    assert_eq!(
        text(response).await,
        r#"{"detail":"code_challenge is required"}"#
    );
}

#[tokio::test]
async fn the_get_refuses_a_wrong_response_type() {
    let query = "response_type=token&redirect_uri=http://127.0.0.1:1456/callback&code_challenge=c";
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(query)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"response_type must be 'code'"}"#
    );
}

#[tokio::test]
async fn the_get_of_a_request_without_a_query_names_the_response_type() {
    let response = send(app(AuthCodeStore::new(), None, None), get(AUTHORIZE_PATH)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"response_type must be 'code'"}"#
    );
}

#[tokio::test]
async fn the_get_refuses_a_non_loopback_redirect_uri() {
    let query = "response_type=code&redirect_uri=https://example.com/cb&code_challenge=c";
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(query)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = text(response).await;
    assert!(body.contains(r#""error":"invalid_request""#));
    assert!(body.contains("redirect_uri must use a loopback host"));
}

#[tokio::test]
async fn a_repeated_query_parameter_keeps_the_last_value() {
    let query =
        format!("response_type=token&response_type=code&redirect_uri={CALLBACK}&code_challenge=c");
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(&query)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_empty_query_parameter_counts_as_present() {
    let query = "response_type=code&redirect_uri=&code_challenge=";
    let response = send(
        app(AuthCodeStore::new(), None, None),
        get(&authorize_uri(query)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"redirect_uri is required"}"#
    );
}

#[tokio::test]
async fn the_post_without_a_cookie_is_login_required() {
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(text(response).await, r#"{"detail":"login_required"}"#);
}

#[tokio::test]
async fn the_post_without_a_master_key_is_login_required() {
    let response = send(
        app(AuthCodeStore::new(), None, None),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, None),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(text(response).await, r#"{"detail":"login_required"}"#);
}

#[tokio::test]
async fn a_signed_in_form_redirects_to_the_callback_with_code_and_state() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(header_of(&response, "content-type"), None);
    let location = header_of(&response, "location").unwrap();
    assert!(location.starts_with("http://127.0.0.1:1456/callback?code="));
    assert!(location.ends_with("&state="));
}

#[tokio::test]
async fn the_stored_code_belongs_to_the_cookie_user() {
    let consent = Arc::new(ConsentState {
        codes: AuthCodeStore::new(),
        master_key: Some(SecretString::from(MASTER_KEY)),
        authenticator: keyring(true),
    });
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        router_with(Arc::clone(&consent)),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    let location = header_of(&response, "location").unwrap();
    let code = location
        .split_once("?code=")
        .unwrap()
        .1
        .split_once("&state=")
        .unwrap()
        .0;
    assert_eq!(consent.codes.get(code).unwrap().user_id, "user-1");
}

#[tokio::test]
async fn the_post_accepts_the_username_password_login() {
    let claims = CookieClaims {
        login_method: "username_password",
        ..ui_claims()
    };
    let cookies = cookie(&claims, MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn the_post_refuses_a_byok_session_token() {
    let claims = CookieClaims {
        token_type: Some("byok_session"),
        ..ui_claims()
    };
    let cookies = cookie(&claims, MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(text(response).await, r#"{"detail":"login_required"}"#);
}

#[tokio::test]
async fn the_post_refuses_an_expired_cookie() {
    let claims = CookieClaims {
        exp: 0,
        ..ui_claims()
    };
    let cookies = cookie(&claims, MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_post_refuses_a_cookie_signed_with_another_key() {
    let cookies = cookie(&ui_claims(), "sk-other-master");
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_post_refuses_a_revoked_session_key() {
    let claims = CookieClaims {
        key: Some("sk-revoked"),
        ..ui_claims()
    };
    let cookies = cookie(&claims, MASTER_KEY);
    let response = send(
        app(AuthCodeStore::new(), Some(MASTER_KEY), keyring(false)),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_post_lets_a_live_session_key_through() {
    let claims = CookieClaims {
        key: Some("sk-live"),
        ..ui_claims()
    };
    let cookies = cookie(&claims, MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn the_post_answers_422_when_a_required_field_is_missing() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let body = format!("redirect_uri={CALLBACK}&code_challenge=challenge");
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &body, FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        header_of(&response, "content-type"),
        Some("application/json")
    );
    assert_eq!(
        text(response).await,
        r#"{"detail":[{"type":"missing","loc":["body","api_key"],"msg":"Field required","input":null}]}"#
    );
}

#[tokio::test]
async fn the_post_lists_every_missing_field_in_declaration_order() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(
            AUTHORIZE_PATH,
            "client_id=client-1",
            FORM_TYPE,
            Some(&cookies),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        text(response).await,
        concat!(
            r#"{"detail":[{"type":"missing","loc":["body","redirect_uri"],"msg":"Field required","input":null},"#,
            r#"{"type":"missing","loc":["body","code_challenge"],"msg":"Field required","input":null},"#,
            r#"{"type":"missing","loc":["body","api_key"],"msg":"Field required","input":null}]}"#
        )
    );
}

#[tokio::test]
async fn the_post_answers_422_for_a_body_that_is_not_a_form() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let request = Request::builder()
        .method("POST")
        .uri(AUTHORIZE_PATH)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &cookies)
        .body(Body::from(
            r#"{"redirect_uri": "http://127.0.0.1:1456/callback"}"#,
        ))
        .unwrap();
    let response = send(signed_in(AuthCodeStore::new()), request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn the_session_check_precedes_the_form_check() {
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(
            AUTHORIZE_PATH,
            "client_id=client-1",
            FORM_TYPE,
            Some("token=not-a-jwt"),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_repeated_form_field_keeps_the_last_value() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let body = format!(
        "redirect_uri=https://example.com/cb&redirect_uri={CALLBACK}&code_challenge=challenge&api_key=sk-user"
    );

    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &body, FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn the_post_refuses_a_form_with_too_many_fields() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let body = (0..=MAX_FORM_FIELDS)
        .map(|index| format!("field{index}=1"))
        .collect::<Vec<_>>()
        .join("&");
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &body, FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"Too many fields. Maximum number of fields is 1000."}"#
    );
}

#[tokio::test]
async fn the_post_refuses_a_non_loopback_redirect_uri() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let body = "redirect_uri=https://example.com/cb&code_challenge=challenge&api_key=sk-user";
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, body, FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        text(response)
            .await
            .contains("redirect_uri must use a loopback host")
    );
}

#[tokio::test]
async fn the_post_refuses_a_challenge_method_that_is_not_s256() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let body = format!(
        "redirect_uri={CALLBACK}&code_challenge=challenge&code_challenge_method=plain&api_key=sk-user"
    );
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &body, FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        text(response).await,
        r#"{"detail":"Only S256 code_challenge_method is supported"}"#
    );
}

#[tokio::test]
async fn a_multipart_form_redirects_to_the_callback() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(
            AUTHORIZE_PATH,
            MULTIPART_FORM,
            "multipart/form-data; boundary=boundary-1",
            Some(&cookies),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = header_of(&response, "location").unwrap();
    assert!(location.starts_with("http://127.0.0.1:1456/callback?code="));
}

#[tokio::test]
async fn the_post_defaults_a_missing_challenge_method_to_s256() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
}

#[tokio::test]
async fn the_post_answers_503_at_capacity() {
    let codes = AuthCodeStore::new();
    (0..AUTH_CODES_MAX_SIZE).for_each(|index| codes.insert(format!("seed-{index}"), live_seed()));
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        signed_in(codes),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        text(response).await,
        r#"{"detail":"Too many pending authorization flows"}"#
    );
}

#[tokio::test]
async fn the_post_uses_an_ampersand_when_the_redirect_uri_has_a_query() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let body = "redirect_uri=http%3A%2F%2F127.0.0.1%3A1456%2Fcallback%3Ftenant%3Dacme&code_challenge=challenge&api_key=sk-user";
    let response = send(
        signed_in(AuthCodeStore::new()),
        post(AUTHORIZE_PATH, body, FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = header_of(&response, "location").unwrap();
    assert!(location.starts_with("http://127.0.0.1:1456/callback?tenant=acme&code="));
}

fn assembled_router() -> Router {
    let mut config = empty_config();
    config.general_settings.master_key = Some(SecretString::from(MASTER_KEY));
    router(
        state_for(&config),
        Arc::new(AtomicBool::new(false)),
        Router::new(),
        Router::new(),
    )
}

#[tokio::test]
async fn the_assembled_router_serves_the_consent_get_outside_the_auth_layers() {
    let response = send(assembled_router(), get(&authorize_uri(&valid_query()))).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_of(&response, "content-type"),
        Some("text/html; charset=utf-8")
    );
}

#[tokio::test]
async fn the_assembled_router_accepts_the_signed_consent_post_outside_the_auth_layers() {
    let cookies = cookie(&ui_claims(), MASTER_KEY);
    let response = send(
        assembled_router(),
        post(AUTHORIZE_PATH, &valid_form(), FORM_TYPE, Some(&cookies)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = header_of(&response, "location").unwrap();
    assert!(location.starts_with("http://127.0.0.1:1456/callback?code="));
}
