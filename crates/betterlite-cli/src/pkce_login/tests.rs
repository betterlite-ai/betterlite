#![cfg(test)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::Duration;

use super::{
    Entropy, Http, HttpBody, HttpResponse,
    callback::LoopbackServer,
    discovery::{discover_cli_auth, origin},
    models::{CallbackCode, CallbackOutcome, CliAuthContract, PkceFailure},
    oauth::{RedeemCodeRequest, pkce_pair, redeem_code, register_client},
    revoke::{RevocationUnavailableOrFailure, revoke_credential},
};

struct FixedEntropy;

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
}

impl Http for ScriptedHttp {
    fn get(&self, _: &str, _: u64) -> Result<HttpResponse, PkceFailure> {
        Ok(self.response.clone())
    }

    fn post(&self, _: &str, _: HttpBody, _: u64, _: bool) -> Result<HttpResponse, PkceFailure> {
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
        code: "code".to_owned(),
        verifier: "verifier".to_owned(),
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
    let result = discover_cli_auth("https://example.test", &ScriptedHttp { response });
    result.unwrap_err();
}

#[test]
fn callback_validation_waits_for_matching_state() {
    let (sender, receiver) = sync_channel(1);
    let mut server = LoopbackServer::bind("state").expect("bind");
    let address = server.local_addr();
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
            sender.send(size).expect("send");
            let mut next_stream = TcpStream::connect(address).expect("connect");
            next_stream
                .write_all(
                    b"GET /callback?state=state&code=good HTTP/1.1\r\nHost: localhost\r\n\r\n",
                )
                .expect("write");
        });
        server.wait(Duration::from_secs(2)).expect("outcome")
    });
    assert!(receiver.recv().expect("response") > 0);
    assert_eq!(
        outcome,
        CallbackOutcome::Code(CallbackCode {
            code: "good".to_owned()
        })
    );
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
    let client = register_client(
        &contract(),
        "http://127.0.0.1:1/callback",
        &ScriptedHttp { response },
    )
    .expect("registered");
    assert_eq!(client, "client");
}

#[test]
fn token_requests_capture_expiry() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":30,"refresh_token":"refresh"}"#.to_vec(),
    };
    let credential =
        redeem_code(&contract(), &redemption(), &ScriptedHttp { response }, 100).expect("token");
    assert_eq!(credential.expires_at, 130);
}

#[test]
fn revoke_503_is_unavailable() {
    let response = HttpResponse {
        status: 503,
        headers: Vec::new(),
        body: b"busy".to_vec(),
    };
    assert!(matches!(
        revoke_credential(&contract(), "client", "refresh", &ScriptedHttp { response }),
        Err(RevocationUnavailableOrFailure::Unavailable(_))
    ));
}

#[test]
fn expires_rejects_non_positive_lifetime() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"access_token":"access","expires_in":0,"refresh_token":"refresh"}"#.to_vec(),
    };
    redeem_code(&contract(), &redemption(), &ScriptedHttp { response }, 100).unwrap_err();
}

#[test]
fn error_detail_is_bounded() {
    let response = HttpResponse {
        status: 400,
        headers: Vec::new(),
        body: br#"{"error_description":"denied"}"#.to_vec(),
    };
    let error = redeem_code(&contract(), &redemption(), &ScriptedHttp { response }, 100)
        .expect_err("failure");
    assert_eq!(error.reason, "denied");
}

#[test]
fn http_timeout_is_passed_to_transport() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: serde_json::to_vec(&contract()).expect("json"),
    };
    let result = discover_cli_auth("https://example.test", &ScriptedHttp { response });
    result.unwrap();
}
