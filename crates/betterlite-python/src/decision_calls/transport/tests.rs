#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_provider::opensandbox::helpers::{
    OpenSandboxError, RequestTimeout, SandboxHttp, SandboxRequest,
};
use http::{HeaderMap, Method};
use url::Url;

use super::ProxiedOpenSandboxHttp;
use crate::completion::setup::outbound_client;
use betterlite_http::proxy::outbound::RequestBody;

fn request(url: &str, body: &[u8], timeout: Option<RequestTimeout>) -> SandboxRequest {
    SandboxRequest {
        method: Method::POST,
        url: Url::parse(url).unwrap(),
        headers: HeaderMap::new(),
        body: body.to_vec(),
        timeout,
    }
}

fn http() -> ProxiedOpenSandboxHttp {
    ProxiedOpenSandboxHttp {
        client: outbound_client(&BTreeMap::new()).unwrap(),
    }
}

#[test]
fn outbound_preserves_request_fields_and_uses_the_read_deadline() {
    let proxied = http();
    let outbound = proxied.outbound(request(
        "https://sandbox.example/run",
        b"{\"code\":\"print(1)\"}",
        Some(RequestTimeout {
            connect: None,
            read: Some(Duration::from_secs(3)),
            write: None,
            pool: None,
        }),
    ));
    assert_eq!(outbound.method, Method::POST);
    assert_eq!(outbound.url.as_str(), "https://sandbox.example/run");
    assert_eq!(outbound.deadline, Duration::from_secs(3));
    assert!(matches!(
        outbound.body,
        RequestBody::Json(body) if body == b"{\"code\":\"print(1)\"}"
    ));
}

#[tokio::test]
async fn send_reports_a_connection_failure() {
    let result = http().send(request("http://127.0.0.1:0/", b"", None)).await;
    assert!(matches!(result, Err(OpenSandboxError::Transport { .. })));
}
