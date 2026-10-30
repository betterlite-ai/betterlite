#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::park_timeout;
use std::time::Duration;
use std::time::Instant;

use super::{
    Clock, Entropy, Http, HttpBody, HttpResponse,
    callback::LoopbackServer,
    discovery::{discover_cli_auth, origin},
    models::{CallbackCode, CallbackOutcome, CliAuthContract, PkceFailure},
    oauth::{
        RedeemCodeRequest, RefreshCredentialRequest, pkce_pair, redeem_code, refresh_credential,
        register_client,
    },
    revoke::{RevocationUnavailableOrFailure, revoke_credential},
};

struct FixedEntropy;

struct FixedClock(Instant);

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

impl Entropy for FixedEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure> {
        for byte in bytes {
            *byte = 1;
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ScriptedHttp {
    response: HttpResponse,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

#[derive(Clone)]
struct RecordedRequest {
    url: String,
    body: HttpBody,
    timeout: u64,
    follow_redirects: bool,
}

#[derive(serde::Deserialize)]
struct RegistrationPayload {
    client_name: String,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
}

impl ScriptedHttp {
    fn new(response: HttpResponse) -> Self {
        Self {
            response,
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("requests").clone()
    }
}

impl Http for ScriptedHttp {
    fn get(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        self.requests
            .lock()
            .map_err(|_poisoned| PkceFailure::Protocol("request recorder was poisoned".to_owned()))?
            .push(RecordedRequest {
                url: url.to_owned(),
                body: HttpBody::Json(Vec::new()),
                timeout: timeout_seconds,
                follow_redirects: false,
            });
        Ok(self.response.clone())
    }

    fn post(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        self.requests
            .lock()
            .map_err(|_poisoned| PkceFailure::Protocol("request recorder was poisoned".to_owned()))?
            .push(RecordedRequest {
                url: url.to_owned(),
                body,
                timeout: timeout_seconds,
                follow_redirects,
            });
        Ok(self.response.clone())
    }
}

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
    assert_eq!(verifier.len(), 86);
    assert_eq!(challenge, "d-tPkxk37XPMxus_-J6yZXGjoN4TOADR8GPQl-whssA");
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
            sender
                .send(String::from_utf8(response[..size].to_vec()).expect("response UTF-8"))
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
#[case(307)]
#[case(308)]
fn register_client_rejects_redirect(#[case] status: u16) {
    let http = ScriptedHttp::new(HttpResponse {
        status,
        headers: Vec::new(),
        body: Vec::new(),
    });
    register_client(&contract(), "http://127.0.0.1:1/callback", &http).unwrap_err();
}

#[test]
fn token_requests_capture_expiry() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":30,"refresh_token":"refresh"}"#.to_vec(),
    };
    let http = ScriptedHttp::new(response);
    let credential = redeem_code(&contract(), &redemption(), &http, 100).expect("token");
    assert_eq!(credential.expires_at, 130);
    let request = http.requests().pop().expect("token request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
    assert_eq!(
        request.body,
        HttpBody::Form(vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("code".to_owned(), "code".to_owned()),
            (
                "redirect_uri".to_owned(),
                "http://127.0.0.1:1/callback".to_owned()
            ),
            ("client_id".to_owned(), "client".to_owned()),
            ("code_verifier".to_owned(), "verifier".to_owned()),
            ("resource".to_owned(), "https://example.test".to_owned()),
        ])
    );
}

#[rstest]
#[case(307)]
#[case(308)]
fn token_request_rejects_redirect(#[case] status: u16) {
    let http = ScriptedHttp::new(HttpResponse {
        status,
        headers: Vec::new(),
        body: Vec::new(),
    });
    redeem_code(&contract(), &redemption(), &http, 100).unwrap_err();
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
        100,
    )
    .expect("refresh");
    let request = http.requests().pop().expect("refresh request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
    assert_eq!(
        request.body,
        HttpBody::Form(vec![
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("refresh_token".to_owned(), "refresh token\0".to_owned()),
            ("client_id".to_owned(), "client".to_owned()),
            ("resource".to_owned(), "https://example.test".to_owned()),
        ])
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
        revoke_credential(&contract(), "client", "refresh", &http),
        Err(RevocationUnavailableOrFailure::Unavailable(_))
    ));
    let request = http.requests().pop().expect("revocation request");
    assert_eq!(request.timeout, 15);
    assert!(!request.follow_redirects);
}

#[rstest]
#[case(307)]
#[case(308)]
fn revoke_rejects_redirect(#[case] status: u16) {
    let http = ScriptedHttp::new(HttpResponse {
        status,
        headers: Vec::new(),
        body: Vec::new(),
    });
    revoke_credential(&contract(), "client", "refresh", &http).unwrap_err();
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
        100,
    )
    .unwrap_err();
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
        100,
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
        100,
    )
    .expect_err("failure");
    assert_eq!(error.to_string().chars().count(), 200);
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
