use axum::http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use crate::cyberark::transport::{ConjurRequest, ConjurTransport as _, HttpMethod};
use crate::secret_read_dispatch::conjur_http::ConjurHttpTransport;
use crate::secret_read_dispatch::http_transport::HttpError;

use super::super::http_server::{Recorded, serve_fixed};

fn header<'record>(request: &'record Recorded, name: &str) -> Option<&'record str> {
    let value = request.headers.get(name)?;
    value.to_str().ok()
}

#[tokio::test]
async fn conjur_http_transport_sends_the_authorization_header() {
    let (server, recorder) = serve_fixed(200, "cyberark-value");
    let transport = ConjurHttpTransport::default();
    let authorization = SecretString::from("Token token=\"abc\"");
    let response = transport
        .send(ConjurRequest {
            method: HttpMethod::Get,
            url: server.url("/secrets/default/variable/db"),
            authorization: Some(&authorization),
            content_type: None,
            body: None,
            verify_tls: true,
            client_certificate: None,
        })
        .await
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "cyberark-value");
    let requests = recorder.all();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(
        header(&requests[0], "authorization"),
        Some("Token token=\"abc\"")
    );
}

#[tokio::test]
async fn conjur_http_transport_posts_the_body_and_content_type() {
    let (server, recorder) = serve_fixed(200, "authenticated");
    let transport = ConjurHttpTransport::default();
    let body = SecretString::from("secret-api-key");
    let response = transport
        .send(ConjurRequest {
            method: HttpMethod::Post,
            url: server.url("/authn/default/admin/authenticate"),
            authorization: None,
            content_type: Some("application/json"),
            body: Some(&body),
            verify_tls: false,
            client_certificate: None,
        })
        .await
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    let requests = recorder.all();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].body, "secret-api-key");
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/json")
    );
}

#[tokio::test]
async fn conjur_http_transport_rejects_an_oversize_reply() {
    let (server, _) = serve_fixed(200, &"x".repeat(1024 * 1024 + 1));
    let transport = ConjurHttpTransport::default();
    let error = transport
        .send(ConjurRequest {
            method: HttpMethod::Get,
            url: server.url("/secrets/default/variable/db"),
            authorization: None,
            content_type: None,
            body: None,
            verify_tls: true,
            client_certificate: None,
        })
        .await
        .expect_err("an oversize reply should fail");
    assert!(matches!(error, HttpError::ResponseTooLarge { .. }));
}

#[test]
fn conjur_http_transport_reads_blocking() {
    let (server, recorder) = serve_fixed(200, "blocking-body");
    let transport = ConjurHttpTransport::default();
    let response = transport
        .send_blocking(ConjurRequest {
            method: HttpMethod::Get,
            url: server.url("/secrets/default/variable/db"),
            authorization: None,
            content_type: None,
            body: None,
            verify_tls: true,
            client_certificate: None,
        })
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "blocking-body");
    assert_eq!(recorder.all().len(), 1);
}
