use std::sync::Arc;
use std::time::Duration;

use axum::http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use crate::secret_read_dispatch::http_transport::{HttpError, HttpExchange, TlsVerification};
use crate::secret_read_dispatch::reqwest_exchange::ReqwestExchange;
use crate::secret_read_dispatch::vault_http::VaultHttpTransport;
use crate::vault::transport::{VaultMethod, VaultRequest, VaultTransport as _};

use super::super::http_server::{Recorded, serve_fixed, serve_redirect, serve_slow};

const TOKEN: &str = "vault-token";

fn exchange() -> Arc<dyn HttpExchange> {
    Arc::new(ReqwestExchange::default())
}

fn transport() -> VaultHttpTransport {
    VaultHttpTransport {
        exchange: exchange(),
        tls: TlsVerification::Enabled,
    }
}

pub(super) fn request(url: String) -> VaultRequest {
    VaultRequest {
        method: VaultMethod::Get,
        url,
        token: Some(SecretString::from(TOKEN)),
        namespace: None,
        body: None,
        client_certificate: None,
        timeout: Duration::from_secs(5),
        follow_redirects: true,
        max_response_bytes: 1024,
    }
}

fn header<'record>(request: &'record Recorded, name: &str) -> Option<&'record str> {
    request.headers.get(name)?.to_str().ok()
}

#[tokio::test]
async fn vault_transport_sends_the_token_and_namespace() {
    let (server, recorder) = serve_fixed(200, "{\"ok\":true}");
    let transport = transport();
    let mut call = request(server.url("/v1/secret/data/db"));
    call.namespace = Some("team".to_owned());
    let response = transport.send(call).await.expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "{\"ok\":true}");
    let requests = recorder.all();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].path, "/v1/secret/data/db");
    assert_eq!(header(&requests[0], "x-vault-token"), Some(TOKEN));
    assert_eq!(header(&requests[0], "x-vault-namespace"), Some("team"));
}

#[tokio::test]
async fn vault_transport_posts_a_json_body() {
    let (server, recorder) = serve_fixed(200, "{}");
    let transport = transport();
    let mut call = request(server.url("/v1/auth/approle/login"));
    call.method = VaultMethod::Post;
    call.body = Some(SecretString::from("{\"role_id\":\"id\"}"));
    let response = transport.send(call).await.expect("a successful reply");
    assert_eq!(response.status, 200);
    let requests = recorder.all();
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].body, "{\"role_id\":\"id\"}");
    assert_eq!(
        header(&requests[0], "content-type"),
        Some("application/json")
    );
}

#[tokio::test]
async fn vault_transport_rejects_an_oversize_reply() {
    let (server, _) = serve_fixed(200, &"x".repeat(64));
    let transport = transport();
    let mut call = request(server.url("/v1/secret/data/db"));
    call.max_response_bytes = 16;
    let error = transport
        .send(call)
        .await
        .expect_err("an oversize reply should fail");
    assert!(matches!(error, HttpError::ResponseTooLarge { limit: 16 }));
}

#[tokio::test]
async fn vault_transport_times_out() {
    let (server, _) = serve_slow(Duration::from_millis(500), "{}");
    let transport = transport();
    let mut call = request(server.url("/v1/secret/data/db"));
    call.timeout = Duration::from_millis(50);
    let error = transport
        .send(call)
        .await
        .expect_err("a slow reply should time out");
    assert!(matches!(error, HttpError::Timeout));
}

#[tokio::test]
async fn vault_transport_honours_follow_redirects() {
    let (server, recorder) = serve_redirect("/second", "moved", "final");
    let transport = transport();
    let followed = transport
        .send(request(server.url("/first")))
        .await
        .expect("a followed redirect");
    assert_eq!(followed.status, 200);
    assert_eq!(followed.body.expose_secret(), "final");
    let mut unfollowed_call = request(server.url("/first"));
    unfollowed_call.follow_redirects = false;
    let unfollowed = transport
        .send(unfollowed_call)
        .await
        .expect("an unfollowed redirect");
    assert_eq!(unfollowed.status, 302);
    assert_eq!(unfollowed.body.expose_secret(), "moved");
    assert_eq!(recorder.all().len(), 3);
}

#[test]
fn vault_transport_reads_blocking() {
    let (server, recorder) = serve_fixed(200, "blocking-body");
    let transport = transport();
    let response = transport
        .send_blocking(request(server.url("/v1/secret/data/db")))
        .expect("a successful reply");
    assert_eq!(response.status, 200);
    assert_eq!(response.body.expose_secret(), "blocking-body");
    assert_eq!(recorder.all().len(), 1);
}

#[tokio::test]
async fn vault_transport_refuses_blocking_inside_a_runtime() {
    let transport = transport();
    let error = transport
        .send_blocking(request("http://127.0.0.1:1/v1/secret/data/db".to_owned()))
        .expect_err("blocking inside a runtime should fail");
    assert!(matches!(error, HttpError::BlockingInAsyncRuntime));
}
