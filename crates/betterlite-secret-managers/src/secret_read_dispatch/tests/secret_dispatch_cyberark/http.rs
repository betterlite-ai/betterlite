use secrecy::{ExposeSecret, SecretString};

use crate::cyberark::transport::{ConjurRequest, ConjurTransport as _, HttpMethod as ConjurMethod};
use crate::secret_read_dispatch::conjur_http::ConjurHttpTransport;
use crate::secret_read_dispatch::http_transport::{self, HttpError, HttpMethod, RequestHeader};

use super::super::exchange::{header, reply, scripted};

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

#[tokio::test]
async fn conjur_http_transport_sends_the_authorization_header() {
    let (exchange, recorder) = scripted(vec![Ok(reply(200, b"cyberark-value"))]);
    let transport = ConjurHttpTransport { exchange };
    let authorization = SecretString::from("Token token=\"abc\"");
    let url = "http://127.0.0.1:8080/secrets/default/variable/db".to_owned();
    let response = transport
        .send(request(url, Some(&authorization)))
        .await
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "cyberark-value");
    let requests = recorder.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, HttpMethod::Get);
    assert_eq!(
        header(&requests[0], "authorization"),
        Some("Token token=\"abc\"")
    );
    let sent = requests[0]
        .headers
        .iter()
        .find(|(name, _)| *name == "authorization")
        .map(|(_, value)| value);
    assert!(matches!(sent, Some(RequestHeader::Secret(_))));
    assert_eq!(requests[0].timeout, http_transport::DEFAULT_TIMEOUT);
    assert_eq!(
        requests[0].max_response_bytes,
        http_transport::MAX_RESPONSE_BYTES
    );
    assert!(requests[0].follow_redirects);
    assert!(requests[0].verify_tls);
}

#[tokio::test]
async fn conjur_http_transport_posts_the_body_and_content_type() {
    let (exchange, recorder) = scripted(vec![Ok(reply(200, b"authenticated"))]);
    let transport = ConjurHttpTransport { exchange };
    let body = SecretString::from("secret-api-key");
    let response = transport
        .send(ConjurRequest {
            method: ConjurMethod::Post,
            url: "http://127.0.0.1:8080/authn/default/admin/authenticate".to_owned(),
            authorization: None,
            content_type: Some("application/x-yaml"),
            body: Some(&body),
            verify_tls: false,
            client_certificate: None,
        })
        .await
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    let requests = recorder.requests();
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(
        requests[0].body.as_ref().map(ExposeSecret::expose_secret),
        Some("secret-api-key")
    );
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/x-yaml")
    );
    assert!(!requests[0].verify_tls);
}

#[tokio::test]
async fn conjur_http_transport_rejects_an_oversize_reply() {
    let body = vec![b'x'; http_transport::MAX_RESPONSE_BYTES + 1];
    let (exchange, _) = scripted(vec![Ok(reply(200, &body))]);
    let transport = ConjurHttpTransport { exchange };
    let error = transport
        .send(request(
            "http://127.0.0.1:8080/secrets/default/variable/db".to_owned(),
            None,
        ))
        .await
        .expect_err("an oversize reply should fail");
    assert!(matches!(
        error,
        HttpError::ResponseTooLarge { limit } if limit == http_transport::MAX_RESPONSE_BYTES
    ));
}

#[test]
fn conjur_http_transport_reads_blocking() {
    let (exchange, recorder) = scripted(vec![Ok(reply(200, b"blocking-body"))]);
    let transport = ConjurHttpTransport { exchange };
    let response = transport
        .send_blocking(request(
            "http://127.0.0.1:8080/secrets/default/variable/db".to_owned(),
            None,
        ))
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "blocking-body");
    assert_eq!(recorder.requests().len(), 1);
}
