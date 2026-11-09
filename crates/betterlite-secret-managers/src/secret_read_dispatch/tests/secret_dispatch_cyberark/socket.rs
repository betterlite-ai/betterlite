use std::sync::Arc;

use axum::http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use crate::cyberark::transport::{ConjurRequest, ConjurTransport as _, HttpMethod as ConjurMethod};
use crate::secret_read_dispatch::conjur_http::ConjurHttpTransport;
use crate::secret_read_dispatch::http_transport::{self, HttpError, HttpExchange};
use crate::secret_read_dispatch::reqwest_exchange::ReqwestExchange;

use super::super::http_server::{Recorded, serve_fixed};

fn exchange() -> Arc<dyn HttpExchange> {
    Arc::new(ReqwestExchange::default())
}

fn transport() -> ConjurHttpTransport {
    ConjurHttpTransport {
        exchange: exchange(),
    }
}

fn request(url: String, authorization: Option<&SecretString>) -> ConjurRequest<'_> {
    ConjurRequest {
        method: ConjurMethod::Get,
        url,
        authorization,
        content_type: None,
        body: None,
        verify_tls: true,
        client_certificate: None,
    }
}

fn header<'record>(request: &'record Recorded, name: &str) -> Option<&'record str> {
    request.headers.get(name)?.to_str().ok()
}

#[tokio::test]
async fn conjur_transport_sends_the_authorization_header() {
    let (server, recorder) = serve_fixed(200, "cyberark-value");
    let transport = transport();
    let authorization = SecretString::from("Token token=\"abc\"");
    let response = transport
        .send(request(
            server.url("/secrets/default/variable/db"),
            Some(&authorization),
        ))
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
async fn conjur_transport_rejects_an_oversize_reply() {
    let (server, _) = serve_fixed(200, &"x".repeat(http_transport::MAX_RESPONSE_BYTES + 1));
    let transport = transport();
    let error = transport
        .send(request(server.url("/secrets/default/variable/db"), None))
        .await
        .expect_err("an oversize reply should fail");
    assert!(matches!(
        error,
        HttpError::ResponseTooLarge { limit } if limit == http_transport::MAX_RESPONSE_BYTES
    ));
}

#[test]
fn conjur_transport_reads_blocking() {
    let (server, recorder) = serve_fixed(200, "blocking-body");
    let transport = transport();
    let response = transport
        .send_blocking(request(server.url("/secrets/default/variable/db"), None))
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "blocking-body");
    assert_eq!(recorder.all().len(), 1);
}
