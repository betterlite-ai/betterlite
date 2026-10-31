#![cfg(test)]

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::VecDeque;
use std::fs;
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::AtomicI64;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::park_timeout;
use std::time::Duration;
use std::time::Instant;

use super::{
    HttpBody, HttpResponse,
    callback::LoopbackServer,
    discovery::{discover_cli_auth, origin},
    models::{CallbackCode, CallbackOutcome, CliAuthContract, PkceFailure},
    oauth::{
        RedeemCodeRequest, RefreshCredentialRequest, authorize_url, pkce_pair, redeem_code,
        refresh_credential, register_client,
    },
    request::{CallbackQuery, RequestRead, parse_request},
    revoke::{RevocationUnavailableOrFailure, revoke_credential},
    wire,
};

mod fixtures;
use fixtures::{
    FailingListener, FixedClock, FixedEntropy, FixedWallClock, RecordedConnection,
    RegistrationPayload, ScriptedHttp, SharedWallClock, SystemClock, form_fields,
    redirect_response,
};

fn contract() -> CliAuthContract {
    CliAuthContract {
        contract_version: 1,
        issuer: "https://example.test".to_owned(),
        authorization_endpoint: "https://example.test/oauth/authorize".to_owned(),
        token_endpoint: "https://example.test/oauth/token".to_owned(),
        registration_endpoint: "https://example.test/oauth/register".to_owned(),
        revocation_endpoint: "https://example.test/oauth/revoke".to_owned(),
        resource: "https://example.test".to_owned(),
        supported_methods: vec!["S256".to_owned()],
    }
}

fn redemption() -> RedeemCodeRequest {
    RedeemCodeRequest {
        client_id: "client".to_owned(),
        redirect_uri: "http://127.0.0.1:1/callback".to_owned(),
        code: SecretString::from("code"),
        verifier: SecretString::from("verifier"),
    }
}

#[test]
fn pkce_pair_has_64_byte_verifier_and_s256_challenge() {
    let (verifier, challenge) = pkce_pair(&mut FixedEntropy).expect("pair");
    assert_eq!(verifier.expose_secret().len(), 86);
    assert_eq!(challenge, "d-tPkxk37XPMxus_-J6yZXGjoN4TOADR8GPQl-whssA");
}

#[test]
fn authorize_url_carries_s256_and_resource() {
    let url = authorize_url(
        &contract(),
        "client-1",
        "http://127.0.0.1:1/callback",
        "state-1",
        "challenge-1",
    );
    assert!(url.starts_with("https://example.test/oauth/authorize?"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("client_id=client-1"));
    assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1%2Fcallback"));
    assert!(url.contains("state=state-1"));
    assert!(url.contains("code_challenge=challenge-1"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("resource=https%3A%2F%2Fexample.test"));
}

#[test]
fn origin_normalizes_default_ports() {
    assert_eq!(
        origin("https://example.test"),
        Some("https://example.test:443".to_owned())
    );
    assert_eq!(
        origin("https://example.test:443/path"),
        Some("https://example.test:443".to_owned())
    );
    assert_eq!(
        origin("https://[::1]/path"),
        Some("https://[::1]:443".to_owned())
    );
}

#[test]
fn discover_rejects_foreign_endpoint() {
    let mut value = contract();
    value.token_endpoint = "https://foreign.test/token".to_owned();
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&value).expect("json"),
    };
    let result = discover_cli_auth("https://example.test", &ScriptedHttp::new(response));
    result.unwrap_err();
}

#[test]
fn discover_rejects_unsupported_contract_version() {
    let body = br#"{
        "contract_version": 2,
        "issuer": "https://example.test",
        "authorization_endpoint": "https://example.test/oauth/authorize",
        "token_endpoint": "https://example.test/oauth/token",
        "registration_endpoint": "https://example.test/oauth/register",
        "revocation_endpoint": "https://example.test/oauth/revoke",
        "resource": "https://example.test",
        "code_challenge_methods_supported": ["S256"]
    }"#;
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: body.to_vec(),
    };
    let result = discover_cli_auth("https://example.test", &ScriptedHttp::new(response));
    result.unwrap_err();
}

#[test]
fn discover_rejects_missing_s256() {
    let mut value = contract();
    value.supported_methods = vec!["plain".to_owned()];
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&value).expect("json"),
    };
    let result = discover_cli_auth("https://example.test", &ScriptedHttp::new(response));
    result.unwrap_err();
}

#[test]
fn discover_rejects_issuer_mismatch() {
    let mut value = contract();
    value.issuer = "https://other.test".to_owned();
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&value).expect("json"),
    };
    let result = discover_cli_auth("https://example.test", &ScriptedHttp::new(response));
    result.unwrap_err();
}

#[test]
fn callback_validation_waits_for_matching_state() {
    let (sender, receiver) = sync_channel(1);
    let mut server = LoopbackServer::bind("state").expect("bind");
    let address = server.address;
    let outcome = thread::scope(|scope| {
        scope.spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            stream
                .write_all(
                    b"GET /callback?state=wrong&code=bad HTTP/1.1\r\nHost: localhost\r\n\r\n",
                )
                .expect("write");
            let mut response = [0; 256];
            let size = stream.read(&mut response).expect("read");
            let payload = response.get(..size).expect("read within buffer").to_vec();
            sender
                .send(String::from_utf8(payload).expect("response UTF-8"))
                .expect("send");
            let mut next_stream = TcpStream::connect(address).expect("connect");
            next_stream
                .write_all(
                    b"GET /callback?state=state&code=good HTTP/1.1\r\nHost: localhost\r\n\r\n",
                )
                .expect("write");
        });
        server
            .wait(Duration::from_secs(2), &FixedClock(Instant::now()))
            .expect("outcome")
    });
    assert!(
        receiver
            .recv()
            .expect("response")
            .starts_with("HTTP/1.1 400")
    );
    assert!(matches!(
        outcome,
        CallbackOutcome::Code(CallbackCode { .. })
    ));
}

#[test]
fn callback_idle_connection_then_valid_callback_settles() {
    let server = LoopbackServer::bind("state").expect("bind");
    let address = server.address;
    let (sender, receiver) = sync_channel(1);
    thread::scope(|scope| {
        scope.spawn(move || {
            let _idle = TcpStream::connect(address).expect("idle connect");
            park_timeout(Duration::from_millis(100));
            let mut valid = TcpStream::connect(address).expect("valid connect");
            valid
                .write_all(
                    b"GET /callback?state=state&code=good HTTP/1.1\r\nHost: localhost\r\n\r\n",
                )
                .expect("write");
            sender.send(()).expect("send");
            park_timeout(Duration::from_millis(100));
        });
        let mut callback_server = server;
        let outcome = callback_server
            .wait(Duration::from_secs(10), &FixedClock(Instant::now()))
            .expect("outcome");
        assert!(matches!(outcome, CallbackOutcome::Code(_)));
    });
    receiver.recv().expect("valid callback");
}

#[test]
fn loopback_server_exposes_callback_redirect() {
    let server = LoopbackServer::bind("state").expect("bind");
    assert!(server.redirect_uri().starts_with("http://127.0.0.1:"));
    assert!(server.redirect_uri().ends_with("/callback"));
}

#[test]
fn loopback_wait_surfaces_accept_failure() {
    let listener = FailingListener {
        address: "127.0.0.1:0".parse().expect("address"),
        attempts: Mutex::new(VecDeque::from([Err(ErrorKind::ConnectionAborted)])),
    };
    let mut server = LoopbackServer::with_listener(listener, "state").expect("bind");
    let error = server
        .wait(Duration::from_secs(1), &FixedClock(Instant::now()))
        .unwrap_err();
    assert!(matches!(error, PkceFailure::LoopbackAccept { .. }));
}

#[test]
fn loopback_wait_ignores_aborted_accept_before_valid_callback() {
    let request =
        b"GET /callback?state=state&code=good HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec();
    let listener = FailingListener {
        address: "127.0.0.1:0".parse().expect("address"),
        attempts: Mutex::new(VecDeque::from([
            Err(ErrorKind::ConnectionAborted),
            Ok(Some(RecordedConnection {
                reads: VecDeque::from([Ok(request)]),
                write_result: Ok(()),
            })),
        ])),
    };
    let mut server = LoopbackServer::with_listener(listener, "state").expect("bind");
    let outcome = server
        .wait(Duration::from_secs(1), &FixedClock(Instant::now()))
        .expect("outcome");
    assert!(matches!(outcome, CallbackOutcome::Code(_)));
}

#[test]
fn loopback_write_failure_is_swallowed_and_connection_still_settles() {
    let request =
        b"GET /callback?state=state&code=good HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec();
    let connection = RecordedConnection {
        reads: VecDeque::from([Ok(request)]),
        write_result: Err(ErrorKind::BrokenPipe),
    };
    let listener = FailingListener {
        address: "127.0.0.1:0".parse().expect("address"),
        attempts: Mutex::new(VecDeque::from([Ok(Some(connection))])),
    };
    let mut server = LoopbackServer::with_listener(listener, "state").expect("bind");
    let outcome = server
        .wait(Duration::from_secs(1), &FixedClock(Instant::now()))
        .expect("outcome");
    assert!(matches!(outcome, CallbackOutcome::Code(_)));
}

#[test]
fn loopback_no_pending_connection_keeps_waiting_until_timeout() {
    let listener = FailingListener {
        address: "127.0.0.1:0".parse().expect("address"),
        attempts: Mutex::new(VecDeque::new()),
    };
    let mut server = LoopbackServer::with_listener(listener, "state").expect("bind");
    let error = server
        .wait(Duration::from_millis(20), &SystemClock)
        .unwrap_err();
    assert!(matches!(error, PkceFailure::CallbackTimeout));
}

// A SecretString has no PartialEq, so a parsed code is compared by exposing its secret rather than with `as_deref`.
fn exposed_code(query: &CallbackQuery) -> Option<&str> {
    query
        .code
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret)
}

#[test]
fn callback_request_parses_once_headers_complete_across_reads() {
    let full = b"GET /callback?state=s&code=c HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let split = full.len().checked_div(2).expect("nonzero length");
    let prefix = full.get(..split).expect("split within buffer");
    assert!(matches!(parse_request(prefix), RequestRead::Incomplete));
    let RequestRead::Complete(query) = parse_request(full) else {
        panic!("expected a complete request");
    };
    assert_eq!(query.state.as_deref(), Some("s"));
    assert_eq!(exposed_code(&query), Some("c"));
}

#[test]
fn callback_request_rejects_non_get_method() {
    let request = b"POST /callback HTTP/1.1\r\nHost: localhost\r\n\r\n";
    assert!(matches!(
        parse_request(request),
        RequestRead::UnsupportedMethod
    ));
}

#[test]
fn callback_request_rejects_extra_request_line_tokens() {
    let request = b"GET /callback HTTP/1.1 extra\r\nHost: localhost\r\n\r\n";
    assert!(matches!(parse_request(request), RequestRead::Malformed));
}

#[test]
fn callback_request_rejects_oversized_buffer() {
    let request = vec![b'A'; super::request::MAX_REQUEST_BYTES + 1];
    assert!(matches!(parse_request(&request), RequestRead::TooLarge));
}

#[rstest]
#[case(b"GET /callback HTTP/nonsense\r\nHost: localhost\r\n\r\n")]
#[case(b"GET /callback HTTP/1\r\nHost: localhost\r\n\r\n")]
#[case(b"GET /callback HTTP/1.1.1\r\nHost: localhost\r\n\r\n")]
#[case(b"GET /callback HTTP/one.one\r\nHost: localhost\r\n\r\n")]
fn callback_request_rejects_malformed_version(#[case] request: &[u8]) {
    assert!(matches!(parse_request(request), RequestRead::Malformed));
}

#[test]
fn callback_request_rejects_http_version_2() {
    let request = b"GET /callback HTTP/2.0\r\nHost: localhost\r\n\r\n";
    assert!(matches!(
        parse_request(request),
        RequestRead::UnsupportedVersion
    ));
}

#[test]
fn callback_request_drops_blank_query_values() {
    let request =
        b"GET /callback?state=&state=real&error=&code=good HTTP/1.1\r\nHost: localhost\r\n\r\n";
    let RequestRead::Complete(query) = parse_request(request) else {
        panic!("expected a complete request");
    };
    assert_eq!(query.state.as_deref(), Some("real"));
    assert_eq!(query.error, None);
    assert_eq!(exposed_code(&query), Some("good"));
}

#[test]
fn callback_query_debug_output_never_reveals_code() {
    let RequestRead::Complete(query) = parse_request(
        b"GET /callback?state=s&code=super-secret-code HTTP/1.1\r\nHost: localhost\r\n\r\n",
    ) else {
        panic!("expected a complete request");
    };
    let rendered = format!("{query:?}");
    assert!(!rendered.contains("super-secret-code"));
    let enum_rendered = format!("{:?}", RequestRead::Complete(query));
    assert!(!enum_rendered.contains("super-secret-code"));
}

#[test]
fn callback_unsupported_method_responds_501() {
    let server = LoopbackServer::bind("state").expect("bind");
    let address = server.address;
    let (sender, receiver) = sync_channel(1);
    thread::scope(|scope| {
        scope.spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect");
            stream
                .write_all(b"POST /callback HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .expect("write");
            let mut response = [0; 256];
            let size = stream.read(&mut response).expect("read");
            let payload = response.get(..size).expect("read within buffer").to_vec();
            sender
                .send(String::from_utf8(payload).expect("response UTF-8"))
                .expect("send");
        });
        let mut callback_server = server;
        let error = callback_server
            .wait(Duration::from_millis(200), &SystemClock)
            .unwrap_err();
        assert!(matches!(error, PkceFailure::CallbackTimeout));
    });
    assert!(
        receiver
            .recv()
            .expect("response")
            .starts_with("HTTP/1.1 501")
    );
}

#[test]
fn register_client_sends_public_client_payload() {
    let response = HttpResponse {
        status: 201,
        headers: Vec::new(),
        body: br#"{"client_id":"client"}"#.to_vec(),
    };
    let http = ScriptedHttp::new(response);
    let client =
        register_client(&contract(), "http://127.0.0.1:1/callback", &http).expect("registered");
    assert_eq!(client, "client");
    let request = http.requests().pop().expect("registration request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
    let HttpBody::Json(body) = request.body else {
        panic!("registration body");
    };
    let payload: RegistrationPayload = serde_json::from_slice(&body).expect("payload");
    assert_eq!(payload.client_name, "litellm-cli");
    assert_eq!(
        payload.redirect_uris,
        vec![String::from("http://127.0.0.1:1/callback")]
    );
    assert_eq!(
        payload.grant_types,
        vec![
            String::from("authorization_code"),
            String::from("refresh_token")
        ]
    );
    assert_eq!(payload.response_types, vec![String::from("code")]);
    assert_eq!(payload.token_endpoint_auth_method, "none");
}

#[rstest]
#[case(300)]
#[case(307)]
#[case(308)]
#[case(399)]
fn register_client_rejects_redirect(#[case] status: u16) {
    let redirect_uri = "http://127.0.0.1:1/callback";
    register_client(&contract(), redirect_uri, &redirect_response(status)).unwrap_err();
}

#[test]
fn register_client_rejects_empty_client_id() {
    let response = HttpResponse {
        status: 201,
        headers: Vec::new(),
        body: br#"{"client_id":""}"#.to_vec(),
    };
    register_client(
        &contract(),
        "http://127.0.0.1:1/callback",
        &ScriptedHttp::new(response),
    )
    .unwrap_err();
}

#[test]
fn token_requests_capture_expiry() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":30,"refresh_token":"refresh"}"#.to_vec(),
    };
    let http = ScriptedHttp::new(response);
    let credential =
        redeem_code(&contract(), &redemption(), &http, &FixedWallClock(100)).expect("token");
    assert_eq!(credential.expires_at, 130);
    let request = http.requests().pop().expect("token request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
    assert_eq!(
        form_fields(&request.body),
        vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("code".to_owned(), "code".to_owned()),
            (
                "redirect_uri".to_owned(),
                "http://127.0.0.1:1/callback".to_owned()
            ),
            ("client_id".to_owned(), "client".to_owned()),
            ("code_verifier".to_owned(), "verifier".to_owned()),
            ("resource".to_owned(), "https://example.test".to_owned()),
        ]
    );
}

#[test]
fn expires_at_samples_wall_clock_after_the_response_arrives() {
    let clock = Arc::new(AtomicI64::new(100));
    let http = ScriptedHttp::new(HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":30,"refresh_token":"refresh"}"#.to_vec(),
    })
    .with_clock_advance(Arc::clone(&clock), 20);
    let wall_clock = SharedWallClock(Arc::clone(&clock));
    let credential = redeem_code(&contract(), &redemption(), &http, &wall_clock).expect("token");
    // The clock only reaches 120 once `post` returns; sampling before the call would yield 130.
    assert_eq!(credential.expires_at, 150);
}

#[rstest]
#[case(300)]
#[case(307)]
#[case(308)]
#[case(399)]
fn token_request_rejects_redirect(#[case] status: u16) {
    let transport = redirect_response(status);
    redeem_code(&contract(), &redemption(), &transport, &FixedWallClock(100)).unwrap_err();
}

#[test]
fn token_request_redirect_message_names_the_location() {
    let transport = ScriptedHttp::new(HttpResponse {
        status: 307,
        headers: vec![(
            String::from("Location"),
            String::from("https://attacker.test/collect"),
        )],
        body: Vec::new(),
    });
    let error = redeem_code(&contract(), &redemption(), &transport, &FixedWallClock(100))
        .expect_err("redirect rejected");
    assert!(error.to_string().contains("https://attacker.test/collect"));
}

#[test]
fn refresh_credential_sends_form_fields() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":30,"refresh_token":"refresh"}"#.to_vec(),
    };
    let http = ScriptedHttp::new(response);
    refresh_credential(
        &contract(),
        &RefreshCredentialRequest {
            client_id: "client".to_owned(),
            refresh_token: SecretString::from("refresh token\0"),
        },
        &http,
        &FixedWallClock(100),
    )
    .expect("refresh");
    let request = http.requests().pop().expect("refresh request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
    assert_eq!(
        form_fields(&request.body),
        vec![
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("refresh_token".to_owned(), "refresh token\0".to_owned()),
            ("client_id".to_owned(), "client".to_owned()),
            ("resource".to_owned(), "https://example.test".to_owned()),
        ]
    );
}

#[test]
fn revoke_503_is_unavailable() {
    let response = HttpResponse {
        status: 503,
        headers: Vec::new(),
        body: b"busy".to_vec(),
    };
    let http = ScriptedHttp::new(response);
    assert!(matches!(
        revoke_credential(&contract(), "client", &SecretString::from("refresh"), &http),
        Err(RevocationUnavailableOrFailure::Unavailable(_))
    ));
    let request = http.requests().pop().expect("revocation request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
}

#[rstest]
#[case(300)]
#[case(307)]
#[case(308)]
#[case(399)]
fn revoke_rejects_redirect(#[case] status: u16) {
    revoke_credential(
        &contract(),
        "client",
        &SecretString::from("refresh"),
        &redirect_response(status),
    )
    .unwrap_err();
}

#[test]
fn expires_rejects_non_positive_lifetime() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":0,"refresh_token":"refresh"}"#.to_vec(),
    };
    redeem_code(
        &contract(),
        &redemption(),
        &ScriptedHttp::new(response),
        &FixedWallClock(100),
    )
    .unwrap_err();
}

#[test]
fn expires_rejects_empty_access_token() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"","expires_in":30,"refresh_token":"refresh"}"#.to_vec(),
    };
    redeem_code(
        &contract(),
        &redemption(),
        &ScriptedHttp::new(response),
        &FixedWallClock(100),
    )
    .unwrap_err();
}

#[test]
fn expires_rejects_empty_refresh_token() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":30,"refresh_token":""}"#.to_vec(),
    };
    redeem_code(
        &contract(),
        &redemption(),
        &ScriptedHttp::new(response),
        &FixedWallClock(100),
    )
    .unwrap_err();
}

#[test]
fn expires_in_accepts_numeric_string() {
    let body = br#"{"access_token":"access","expires_in":"30","refresh_token":"refresh"}"#;
    let token = wire::parse_token(body).expect("token");
    assert_eq!(token.expires_in, 30);
}

#[test]
fn expires_in_accepts_integral_float() {
    let body = br#"{"access_token":"access","expires_in":30.0,"refresh_token":"refresh"}"#;
    let token = wire::parse_token(body).expect("token");
    assert_eq!(token.expires_in, 30);
}

#[test]
fn expires_in_accepts_exponent_number() {
    let body = br#"{"access_token":"access","expires_in":3e1,"refresh_token":"refresh"}"#;
    let token = wire::parse_token(body).expect("token");
    assert_eq!(token.expires_in, 30);
}

#[test]
fn expires_in_accepts_underscore_numeric_string() {
    let body = br#"{"access_token":"access","expires_in":"3_600","refresh_token":"refresh"}"#;
    let token = wire::parse_token(body).expect("token");
    assert_eq!(token.expires_in, 3_600);
}

#[test]
fn expires_in_rejects_fractional_float() {
    let body = br#"{"access_token":"access","expires_in":30.5,"refresh_token":"refresh"}"#;
    wire::parse_token(body).unwrap_err();
}

#[test]
fn expires_in_accepts_whitespace_padded_numeric_string() {
    let body = br#"{"access_token":"access","expires_in":" 30 ","refresh_token":"refresh"}"#;
    let token = wire::parse_token(body).expect("token");
    assert_eq!(token.expires_in, 30);
}

#[test]
fn expires_in_accepts_unicode_escaped_numeric_string() {
    // Spells "30" as two JSON unicode escapes, built at run time so this file never embeds one literally.
    let backslash = '\\';
    let body = format!(
        r#"{{"access_token":"access","expires_in":"{backslash}u0033{backslash}u0030","refresh_token":"refresh"}}"#
    );
    let token = wire::parse_token(body.as_bytes()).expect("token");
    assert_eq!(token.expires_in, 30);
}

#[test]
fn expires_in_rejects_whitespace_padded_fractional_string() {
    let body = br#"{"access_token":"access","expires_in":" 30.5 ","refresh_token":"refresh"}"#;
    wire::parse_token(body).unwrap_err();
}

#[test]
fn error_detail_is_bounded() {
    let response = HttpResponse {
        status: 400,
        headers: Vec::new(),
        body: br#"{"error_description":"denied"}"#.to_vec(),
    };
    let error = redeem_code(
        &contract(),
        &redemption(),
        &ScriptedHttp::new(response),
        &FixedWallClock(100),
    )
    .expect_err("failure");
    assert_eq!(error.to_string(), "denied");
}

#[test]
fn error_detail_preserves_invalid_utf8_fallback() {
    let response = HttpResponse {
        status: 400,
        headers: Vec::new(),
        body: vec![0xff; 201],
    };
    let error = redeem_code(
        &contract(),
        &redemption(),
        &ScriptedHttp::new(response),
        &FixedWallClock(100),
    )
    .expect_err("failure");
    assert_eq!(error.to_string().chars().count(), 200);
}

#[test]
fn error_description_empty_falls_through_to_error() {
    let body = br#"{"error_description":"","error":"denied"}"#;
    assert_eq!(wire::error_detail(body), "denied");
}

#[test]
fn error_detail_structured_value_is_stringified() {
    let body = br#"{"error_description":null,"error":null,"detail":{"code":"bad"}}"#;
    assert_eq!(wire::error_detail(body), r#"{"code":"bad"}"#);
}

#[test]
fn error_detail_zero_number_falls_through_to_error() {
    let body = br#"{"error_description":0,"error":"denied"}"#;
    assert_eq!(wire::error_detail(body), "denied");
}

#[test]
fn error_detail_nonzero_number_is_stringified() {
    let body = br#"{"error_description":5}"#;
    assert_eq!(wire::error_detail(body), "5");
}

// Pins the documented deviation from LiteLLM's Python dict-repr fallback; see docs/deviations/B4573.yaml.
#[test]
fn error_detail_all_falsy_falls_back_to_raw_body_not_python_dict_repr() {
    let body = br#"{"other":"x"}"#;
    assert_eq!(wire::error_detail(body), r#"{"other":"x"}"#);
}

#[test]
fn http_debug_output_never_reveals_response_body_bytes() {
    let body = b"super-secret-token-value".to_vec();
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: body.clone(),
    };
    let rendered = format!("{response:?}");
    // Pins the exact Debug output, since a derived impl printing decimal bytes would also pass a mere substring check.
    assert_eq!(
        rendered,
        format!(
            "HttpResponse {{ status: 200, headers: [], body: [{} bytes] }}",
            body.len()
        )
    );
}

#[test]
fn http_debug_output_redacts_response_header_values() {
    let response = HttpResponse {
        status: 200,
        headers: vec![(
            "Set-Cookie".to_owned(),
            "session=super-secret-cookie".to_owned(),
        )],
        body: Vec::new(),
    };
    let rendered = format!("{response:?}");
    assert!(!rendered.contains("super-secret-cookie"));
    assert!(rendered.contains("Set-Cookie"));
    assert!(rendered.contains("[REDACTED]"));
}

#[test]
fn http_debug_output_never_reveals_form_field_values() {
    let body = HttpBody::Form(vec![(
        "refresh_token".to_owned(),
        SecretString::from("super-secret-refresh-value"),
    )]);
    let rendered = format!("{body:?}");
    assert!(!rendered.contains("super-secret-refresh-value"));
}

#[test]
fn http_timeout_is_passed_to_transport() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&contract()).expect("json"),
    };
    let client = ScriptedHttp::new(response);
    let result = discover_cli_auth("https://example.test", &client);
    result.unwrap();
    let request = client.requests().pop().expect("discovery request");
    assert_eq!(
        request.url,
        "https://example.test/.well-known/litellm-cli-auth"
    );
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
}

#[rstest]
fn replays_the_callback_request_corpus(#[files("fuzz/corpus/callback_request/*")] path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else {
        panic!("{}", path.display());
    };
    let _outcome = parse_request(&bytes);
}

#[rstest]
fn replays_the_wire_responses_corpus(#[files("fuzz/corpus/wire_responses/*")] path: PathBuf) {
    let Ok(bytes) = fs::read(&path) else {
        panic!("{}", path.display());
    };
    let _token = wire::parse_token(&bytes);
    let _client = wire::parse_registered_client(&bytes);
    let _contract = wire::parse_contract(&bytes);
    let _detail = wire::error_detail(&bytes);
}
