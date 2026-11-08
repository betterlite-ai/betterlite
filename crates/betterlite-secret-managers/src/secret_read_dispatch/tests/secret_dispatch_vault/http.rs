use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_read_dispatch::http_transport::{HttpError, HttpMethod};
use crate::secret_read_dispatch::vault_http::VaultHttpTransport;
use crate::vault::transport::{VaultMethod, VaultRequest, VaultTransport as _};

use super::super::exchange::{header, reply, scripted};

const TOKEN: &str = "vault-token";

fn request(path: &str) -> VaultRequest {
    VaultRequest {
        method: VaultMethod::Get,
        url: format!("http://127.0.0.1:8200{path}"),
        token: Some(SecretString::from(TOKEN)),
        namespace: None,
        body: None,
        client_certificate: None,
        timeout: Duration::from_secs(5),
        follow_redirects: true,
        max_response_bytes: 1024,
    }
}

#[tokio::test]
async fn vault_http_transport_sends_the_token_and_namespace() {
    let (exchange, recorder) = scripted(vec![Ok(reply(200, b"{\"ok\":true}"))]);
    let transport = VaultHttpTransport { exchange };
    let mut call = request("/v1/secret/data/db");
    call.namespace = Some("team".to_owned());
    let response = transport.send(call).await.expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "{\"ok\":true}");
    let requests = recorder.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, HttpMethod::Get);
    assert_eq!(requests[0].url, "http://127.0.0.1:8200/v1/secret/data/db");
    assert_eq!(header(&requests[0], "x-vault-token"), Some(TOKEN));
    assert_eq!(header(&requests[0], "x-vault-namespace"), Some("team"));
    assert_eq!(requests[0].timeout, Duration::from_secs(5));
    assert!(requests[0].follow_redirects);
    assert!(requests[0].verify_tls);
    assert_eq!(requests[0].max_response_bytes, 1024);
}

#[tokio::test]
async fn vault_http_transport_posts_a_json_body() {
    let (exchange, recorder) = scripted(vec![Ok(reply(200, b"{}"))]);
    let transport = VaultHttpTransport { exchange };
    let mut call = request("/v1/auth/approle/login");
    call.method = VaultMethod::Post;
    call.body = Some(SecretString::from("{\"role_id\":\"id\"}"));
    let response = transport.send(call).await.expect("a successful reply");
    assert_eq!(response.status, 200);
    let requests = recorder.requests();
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(requests[0].body.as_deref(), Some("{\"role_id\":\"id\"}"));
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/json")
    );
}

#[tokio::test]
async fn vault_http_transport_maps_the_delete_method() {
    let (exchange, recorder) = scripted(vec![Ok(reply(204, b""))]);
    let transport = VaultHttpTransport { exchange };
    let mut call = request("/v1/secret/data/db");
    call.method = VaultMethod::Delete;
    let response = transport.send(call).await.expect("a successful reply");
    assert_eq!(response.status, 204);
    assert_eq!(recorder.requests()[0].method, HttpMethod::Delete);
}

#[tokio::test]
async fn vault_http_transport_rejects_an_oversize_reply() {
    let (exchange, _) = scripted(vec![Ok(reply(200, &[b'x'; 64]))]);
    let transport = VaultHttpTransport { exchange };
    let mut call = request("/v1/secret/data/db");
    call.max_response_bytes = 16;
    let error = transport
        .send(call)
        .await
        .expect_err("an oversize reply should fail");
    assert!(matches!(error, HttpError::ResponseTooLarge { limit: 16 }));
}

#[tokio::test]
async fn vault_http_transport_surfaces_an_exchange_failure() {
    let (exchange, _) = scripted(vec![Err(HttpError::Timeout)]);
    let transport = VaultHttpTransport { exchange };
    let error = transport
        .send(request("/v1/secret/data/db"))
        .await
        .expect_err("a timeout should fail");
    assert!(matches!(error, HttpError::Timeout));
}

#[test]
fn vault_http_transport_reads_blocking() {
    let (exchange, recorder) = scripted(vec![Ok(reply(200, b"blocking-body"))]);
    let transport = VaultHttpTransport { exchange };
    let response = transport
        .send_blocking(request("/v1/secret/data/db"))
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "blocking-body");
    assert_eq!(recorder.requests().len(), 1);
}
