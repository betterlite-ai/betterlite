use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};
use url::Url;

use crate::credentials::login::{
    LOGIN_TIMEOUT_SECONDS, LoginError, LoginRequest, pkce_token_record, run_pkce_login,
};
use crate::pkce_login::HttpBody;
use crate::pkce_login::models::{PkceCredential, PkceFailure};

use super::support::{
    FixedEntropy, FixedWallClock, RESOURCE, REVOCATION_ENDPOINT, Reply, SERVER, SystemClock,
    TOKEN_ENDPOINT, contract_body, deliver_callback, form_fields, form_value, response,
    scripted_http, secret, token_body_with_identity,
};

#[derive(serde::Deserialize)]
struct RegistrationPayload {
    client_name: String,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
}

fn json_bytes(body: &HttpBody) -> &[u8] {
    let HttpBody::Json(bytes) = body else {
        panic!("expected a JSON request body");
    };
    bytes
}

fn login_http() -> super::support::ScriptedHttp {
    scripted_http(vec![
        Reply::Response(response(200, &contract_body(SERVER))),
        Reply::Response(response(201, r#"{"client_id":"client-1"}"#)),
        Reply::Response(response(
            200,
            &token_body_with_identity("sk-access", "refresh-1", 3_600, "user-7", "team-3"),
        )),
    ])
}

fn run_login(
    http: &super::support::ScriptedHttp,
    log: &Mutex<Vec<String>>,
    callback: &[(&str, &str)],
    timeout: Duration,
) -> Result<PkceCredential, LoginError> {
    let mut entropy = FixedEntropy;
    let clock = SystemClock;
    let wall = FixedWallClock(1_700_000_000);
    let echo = |message: &str| log.lock().expect("the echo log").push(message.to_owned());
    let opener = |url: &str| deliver_callback(url, callback);
    run_pkce_login(LoginRequest {
        base_url: SERVER,
        http,
        open_browser: &opener,
        echo: &echo,
        entropy: &mut entropy,
        clock: &clock,
        wall_clock: &wall,
        timeout,
    })
}

#[test]
fn run_pkce_login_returns_the_redeemed_credential() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    let credential = run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect("signs in");

    assert_eq!(credential.access_token.expose_secret(), "sk-access");
    assert_eq!(credential.refresh_token.expose_secret(), "refresh-1");
    assert_eq!(credential.expires_at, 1_700_003_600);
    assert_eq!(credential.client_id, "client-1");
    assert_eq!(credential.token_endpoint, TOKEN_ENDPOINT);
    assert_eq!(credential.revocation_endpoint, REVOCATION_ENDPOINT);
    assert_eq!(credential.resource, RESOURCE);
    assert_eq!(credential.user_id.as_deref(), Some("user-7"));
    assert_eq!(credential.team_id.as_deref(), Some("team-3"));
    assert_eq!(http.requests().len(), 3);
}

#[test]
fn run_pkce_login_echoes_the_authorize_url_before_waiting() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect("signs in");

    let echoed = log.lock().expect("the echo log");
    assert_eq!(echoed.len(), 2);
    assert!(echoed[0].starts_with("Opening browser to: "));
    assert_eq!(echoed[1], "Approve the sign-in in your browser. Waiting...");
}

#[test]
fn run_pkce_login_registers_the_loopback_redirect() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect("signs in");

    let requests = http.requests();
    assert_eq!(
        requests[0].url,
        format!("{SERVER}/.well-known/litellm-cli-auth")
    );
    assert_eq!(requests[0].timeout, 15);
    assert_eq!(requests[1].url, format!("{SERVER}/register"));
    let registration: RegistrationPayload =
        serde_json::from_slice(json_bytes(&requests[1].body)).expect("the registration body");
    assert_eq!(registration.client_name, "litellm-cli");
    assert_eq!(
        registration.grant_types,
        vec!["authorization_code", "refresh_token"]
    );
    assert_eq!(registration.response_types, vec!["code"]);
    assert_eq!(registration.token_endpoint_auth_method, "none");
    assert_eq!(registration.redirect_uris.len(), 1);
    let redirect_uri = &registration.redirect_uris[0];
    assert!(redirect_uri.starts_with("http://127.0.0.1:"));
    assert!(redirect_uri.ends_with("/callback"));
}

#[test]
fn run_pkce_login_authorizes_with_s256_and_a_random_state() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect("signs in");

    let echoed = log.lock().expect("the echo log").clone();
    let authorize = echo_url(&echoed[0]);
    assert_eq!(query(&authorize, "response_type"), "code");
    assert_eq!(query(&authorize, "client_id"), "client-1");
    assert_eq!(query(&authorize, "code_challenge_method"), "S256");
    assert_eq!(query(&authorize, "resource"), RESOURCE);
    assert_eq!(query(&authorize, "state"), expected_state());
}

#[test]
fn run_pkce_login_redeems_the_callback_code_with_the_verifier() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect("signs in");

    let requests = http.requests();
    let fields = form_fields(&requests[2].body);
    assert_eq!(requests[2].url, format!("{SERVER}/token"));
    assert_eq!(requests[2].timeout, 15);
    assert!(!requests[2].follow_redirects);
    assert_eq!(form_value(&fields, "grant_type"), "authorization_code");
    assert_eq!(form_value(&fields, "code"), "auth-code");
    assert_eq!(form_value(&fields, "client_id"), "client-1");
    let echoed = log.lock().expect("the echo log").clone();
    let authorize = echo_url(&echoed[0]);
    assert_eq!(
        query(&authorize, "redirect_uri"),
        form_value(&fields, "redirect_uri")
    );
    let verifier = form_value(&fields, "code_verifier");
    assert_eq!(
        query(&authorize, "code_challenge"),
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    );
}

fn echo_url(echoed: &str) -> Url {
    let text = echoed
        .strip_prefix("Opening browser to: ")
        .expect("the authorize echo");
    Url::parse(text).expect("the authorize URL")
}

fn query(url: &Url, name: &str) -> String {
    let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
    params
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn expected_state() -> String {
    URL_SAFE_NO_PAD.encode([1_u8; 32])
}

#[test]
fn run_pkce_login_reports_a_denied_callback() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    let error = run_login(
        &http,
        &log,
        &[("error", "access_denied"), ("error_description", "denied")],
        Duration::from_secs(5),
    )
    .expect_err("the denied sign-in");

    let LoginError::Pkce(PkceFailure::Protocol(message)) = error else {
        panic!("expected the denied sign-in message");
    };
    assert_eq!(message, "sign-in was not approved (access_denied): denied");
    assert_eq!(http.requests().len(), 2);
}

#[test]
fn run_pkce_login_reports_a_denied_callback_without_details() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    let error = run_login(
        &http,
        &log,
        &[("error", "access_denied")],
        Duration::from_secs(5),
    )
    .expect_err("the denied sign-in");

    let LoginError::Pkce(PkceFailure::Protocol(message)) = error else {
        panic!("expected the denied sign-in message");
    };
    assert_eq!(
        message,
        "sign-in was not approved (access_denied): no details"
    );
}

#[test]
fn run_pkce_login_times_out_without_a_callback() {
    let http = login_http();
    let log = Mutex::new(Vec::new());

    let error = run_login(&http, &log, &[], Duration::from_millis(30)).expect_err("the timeout");

    assert!(matches!(
        error,
        LoginError::Pkce(PkceFailure::CallbackTimeout)
    ));
}

#[test]
fn run_pkce_login_stops_when_discovery_fails() {
    let http = scripted_http(vec![Reply::Response(response(404, "missing"))]);
    let log = Mutex::new(Vec::new());

    let error = run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect_err("the discovery failure");

    assert!(matches!(
        error,
        LoginError::Pkce(PkceFailure::DiscoveryStatus { status: 404 })
    ));
    assert_eq!(http.requests().len(), 1);
}

#[test]
fn run_pkce_login_stops_when_registration_fails() {
    let http = scripted_http(vec![
        Reply::Response(response(200, &contract_body(SERVER))),
        Reply::Response(response(500, r#"{"error":"registration_closed"}"#)),
    ]);
    let log = Mutex::new(Vec::new());

    let error = run_login(
        &http,
        &log,
        &[("code", "auth-code")],
        Duration::from_secs(5),
    )
    .expect_err("the registration failure");

    let LoginError::Pkce(PkceFailure::Protocol(message)) = error else {
        panic!("expected the registration failure");
    };
    assert_eq!(message, "client registration failed: registration_closed");
    assert_eq!(http.requests().len(), 2);
}

fn credential(user_id: Option<&str>, team_id: Option<&str>) -> PkceCredential {
    PkceCredential {
        access_token: secret("sk-access"),
        refresh_token: secret("refresh-1"),
        expires_at: 1_700_003_600,
        client_id: String::from("client-1"),
        token_endpoint: TOKEN_ENDPOINT.to_owned(),
        revocation_endpoint: REVOCATION_ENDPOINT.to_owned(),
        resource: RESOURCE.to_owned(),
        user_id: user_id.map(str::to_owned),
        team_id: team_id.map(str::to_owned),
    }
}

#[test]
fn pkce_token_record_maps_the_credential_to_the_stored_shape() {
    let clock = FixedWallClock(1_700_000_000);
    let record = pkce_token_record(
        "https://gateway.example/",
        &credential(Some("user-7"), Some("team-3")),
        &clock,
    );

    assert_eq!(record.base_url, SERVER);
    assert_eq!(
        record.key.as_ref().expect("a key").expose_secret(),
        "sk-access"
    );
    assert_eq!(
        record
            .refresh_token
            .as_ref()
            .expect("a refresh")
            .expose_secret(),
        "refresh-1"
    );
    assert_eq!(record.timestamp.to_bits(), 1_700_000_000.0_f64.to_bits());
    assert_eq!(
        record.expires_at.map(f64::to_bits),
        Some(1_700_003_600.0_f64.to_bits())
    );
}

#[test]
fn pkce_token_record_uses_the_login_placeholders_and_endpoints() {
    let clock = FixedWallClock(1_700_000_000);
    let record = pkce_token_record(SERVER, &credential(Some("user-7"), Some("team-3")), &clock);

    assert_eq!(record.user_id, "user-7");
    assert_eq!(record.user_email, "unknown");
    assert_eq!(record.user_role, "cli");
    assert_eq!(record.auth_header_name, "Authorization");
    assert_eq!(record.jwt_token.expose_secret(), "");
    assert_eq!(record.client_id.as_deref(), Some("client-1"));
    assert_eq!(record.token_endpoint.as_deref(), Some(TOKEN_ENDPOINT));
    assert_eq!(
        record.revocation_endpoint.as_deref(),
        Some(REVOCATION_ENDPOINT)
    );
    assert_eq!(record.resource.as_deref(), Some(RESOURCE));
    assert_eq!(record.team_id.as_deref(), Some("team-3"));
}

#[test]
fn pkce_token_record_defaults_an_absent_user_to_the_cli_placeholder() {
    let clock = FixedWallClock(1_700_000_000);
    let record = pkce_token_record(SERVER, &credential(None, None), &clock);

    assert_eq!(record.user_id, "cli-user");
    assert!(record.team_id.is_none());
    assert_eq!(record.base_url, SERVER);
}

#[test]
fn pkce_token_record_keeps_an_empty_user_id_as_given() {
    let clock = FixedWallClock(1_700_000_000);
    let record = pkce_token_record(SERVER, &credential(Some(""), None), &clock);

    assert_eq!(record.user_id, "");
}

#[test]
fn the_login_timeout_defaults_to_five_minutes() {
    assert_eq!(LOGIN_TIMEOUT_SECONDS, 300);
}
