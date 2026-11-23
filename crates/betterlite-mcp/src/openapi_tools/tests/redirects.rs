use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use http::Method;

use crate::client::sessions::factory::RedirectGuard;
use crate::openapi_tools::transport::{
    OpenApiRequest, OpenApiTransport, OpenApiTransportError, ProxiedTransport,
};

use super::http_server::{TestServer, header_value, ok_json, redirect, status};

fn client() -> Arc<ProxiedClient> {
    let trust = ProxyTrust::default();
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap())
}

fn transport() -> ProxiedTransport {
    ProxiedTransport::new(client())
}

fn get(url: &str) -> OpenApiRequest {
    OpenApiRequest {
        method: Method::GET,
        url: url.to_owned(),
        query: Vec::new(),
        headers: BTreeMap::new(),
        body: None,
        redirect_guard: None,
    }
}

fn with_headers(request: &mut OpenApiRequest, pairs: &[(&str, &str)]) {
    for (name, value) in pairs {
        request
            .headers
            .insert((*name).to_owned(), (*value).to_owned());
    }
}

#[tokio::test]
async fn follows_a_307_with_the_method_and_body() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(307, &location)).await;
    let mut request = get(&source.url("/start"));
    request.method = Method::POST;
    request.body = Some(br#"{"a":1}"#.to_vec());
    let response = transport().send(request).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"{}".to_vec());
    let seen = target.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(seen[0].uri, "/next");
    assert_eq!(seen[0].body, br#"{"a":1}"#.to_vec());
    assert_eq!(
        header_value(&seen[0], "content-type").as_deref(),
        Some("application/json")
    );
}

#[tokio::test]
async fn a_303_turns_a_post_into_a_get_without_a_body() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(303, &location)).await;
    let mut request = get(&source.url("/start"));
    request.method = Method::POST;
    request.body = Some(br#"{"a":1}"#.to_vec());
    let response = transport().send(request).await.unwrap();
    assert_eq!(response.status, 200);
    let seen = target.requests();
    assert_eq!(seen[0].method, Method::GET);
    assert!(seen[0].body.is_empty());
    assert_eq!(header_value(&seen[0], "content-type"), None);
}

#[tokio::test]
async fn a_301_turns_a_post_into_a_get() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(301, &location)).await;
    let mut request = get(&source.url("/start"));
    request.method = Method::POST;
    request.body = Some(br#"{"a":1}"#.to_vec());
    transport().send(request).await.unwrap();
    let seen = target.requests();
    assert_eq!(seen[0].method, Method::GET);
    assert!(seen[0].body.is_empty());
}

#[tokio::test]
async fn a_302_turns_a_put_into_a_get() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(302, &location)).await;
    let mut request = get(&source.url("/start"));
    request.method = Method::PUT;
    request.body = Some(br#"{"a":1}"#.to_vec());
    transport().send(request).await.unwrap();
    assert_eq!(target.requests()[0].method, Method::GET);
}

#[tokio::test]
async fn a_307_keeps_a_put_with_its_body() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(307, &location)).await;
    let mut request = get(&source.url("/start"));
    request.method = Method::PUT;
    request.body = Some(br#"{"a":1}"#.to_vec());
    transport().send(request).await.unwrap();
    let seen = target.requests();
    assert_eq!(seen[0].method, Method::PUT);
    assert_eq!(seen[0].body, br#"{"a":1}"#.to_vec());
}

#[tokio::test]
async fn a_302_keeps_a_get_and_does_not_reapply_the_query() {
    let server = TestServer::start(|recorded| {
        if recorded.uri.starts_with("/start") {
            redirect(302, "/next")
        } else {
            ok_json("{}")
        }
    })
    .await;
    let mut request = get(&server.url("/start"));
    request.query = vec![("q".to_owned(), "1".to_owned())];
    let response = transport().send(request).await.unwrap();
    assert_eq!(response.status, 200);
    let seen = server.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].uri, "/start?q=1");
    assert_eq!(seen[1].uri, "/next");
}

#[tokio::test]
async fn a_redirect_without_a_location_returns_the_response() {
    let server = TestServer::start(|_| status(302)).await;
    let response = transport().send(get(&server.url("/start"))).await.unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn twenty_redirects_are_followed() {
    let hops = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hops);
    let server = TestServer::start(move |_| {
        if counter.fetch_add(1, Ordering::SeqCst) < 20 {
            redirect(302, "/next")
        } else {
            ok_json("{}")
        }
    })
    .await;
    let response = transport().send(get(&server.url("/next"))).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(server.requests().len(), 21);
}

#[tokio::test]
async fn the_twenty_first_redirect_is_refused() {
    let server = TestServer::start(|_| redirect(302, "/loop")).await;
    let error = transport()
        .send(get(&server.url("/loop")))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        OpenApiTransportError::Send(TransportError::TooManyRedirects { limit: 20 })
    ));
    assert_eq!(server.requests().len(), 21);
}

#[tokio::test]
async fn authorization_is_dropped_across_origins() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(307, &location)).await;
    let mut request = get(&source.url("/start"));
    with_headers(&mut request, &[("Authorization", "Bearer secret")]);
    transport().send(request).await.unwrap();
    assert_eq!(
        header_value(&source.requests()[0], "authorization").as_deref(),
        Some("Bearer secret")
    );
    assert_eq!(header_value(&target.requests()[0], "authorization"), None);
}

#[tokio::test]
async fn authorization_survives_a_same_origin_redirect() {
    let server = TestServer::start(|recorded| {
        if recorded.uri.starts_with("/start") {
            redirect(302, "/next")
        } else {
            ok_json("{}")
        }
    })
    .await;
    let mut request = get(&server.url("/start"));
    with_headers(&mut request, &[("Authorization", "Bearer secret")]);
    transport().send(request).await.unwrap();
    let seen = server.requests();
    assert_eq!(
        header_value(&seen[1], "authorization").as_deref(),
        Some("Bearer secret")
    );
}

#[tokio::test]
async fn a_custom_credential_slot_is_dropped_across_origins() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(307, &location)).await;
    let origin = source.url("/start");
    let mut request = get(&origin);
    with_headers(&mut request, &[("X-Cred", "secret")]);
    request.redirect_guard = RedirectGuard::new(&origin, Some("X-Cred"));
    transport().send(request).await.unwrap();
    let first = source.requests();
    assert_eq!(header_value(&first[0], "x-cred").as_deref(), Some("secret"));
    assert_eq!(header_value(&target.requests()[0], "x-cred"), None);
}

#[tokio::test]
async fn a_custom_credential_slot_survives_a_same_origin_redirect() {
    let server = TestServer::start(|recorded| {
        if recorded.uri.starts_with("/start") {
            redirect(302, "/next")
        } else {
            ok_json("{}")
        }
    })
    .await;
    let origin = server.url("/start");
    let mut request = get(&origin);
    with_headers(&mut request, &[("X-Cred", "secret")]);
    request.redirect_guard = RedirectGuard::new(&origin, Some("X-Cred"));
    transport().send(request).await.unwrap();
    let seen = server.requests();
    assert_eq!(header_value(&seen[1], "x-cred").as_deref(), Some("secret"));
}

#[tokio::test]
async fn the_guard_compares_the_slot_without_case() {
    let target = TestServer::start(|_| ok_json("{}")).await;
    let location = target.url("/next");
    let source = TestServer::start(move |_| redirect(307, &location)).await;
    let origin = source.url("/start");
    let mut request = get(&origin);
    with_headers(&mut request, &[("x-cred", "secret")]);
    request.redirect_guard = RedirectGuard::new(&origin, Some("X-Cred"));
    transport().send(request).await.unwrap();
    assert_eq!(header_value(&target.requests()[0], "x-cred"), None);
}
