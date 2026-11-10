use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use http::Method;
use secrecy::SecretString;

use crate::qdrant_semantic_cache::live::LiveQdrantTransport;
use crate::qdrant_semantic_cache::ports::QdrantRequest;

fn client() -> ProxiedClient {
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the proxy policy");
    ProxiedClient::outbound(&Arc::new(policy)).expect("the outbound client")
}

fn search_request() -> QdrantRequest {
    QdrantRequest {
        method: Method::POST,
        path: "/collections/semantic-cache/points/search".to_owned(),
        query: Vec::new(),
        body: Some(br#"{"limit":1}"#.to_vec()),
    }
}

#[test]
fn live_transport_builds_the_search_request() {
    let transport = LiveQdrantTransport::new(client(), "http://qdrant.test".to_owned(), None)
        .expect("the transport");
    let request = transport
        .build_request(search_request())
        .expect("the outbound request");
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "http://qdrant.test/collections/semantic-cache/points/search"
    );
    assert_eq!(
        request
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert!(request.headers.get("api-key").is_none());
}

#[test]
fn live_transport_adds_the_api_key_header() {
    let transport = LiveQdrantTransport::new(
        client(),
        "http://qdrant.test".to_owned(),
        Some(SecretString::from("secret-key")),
    )
    .expect("the transport");
    let request = transport
        .build_request(QdrantRequest {
            method: Method::PUT,
            path: "/collections/semantic-cache/points".to_owned(),
            query: vec![("wait".to_owned(), "true".to_owned())],
            body: Some(br#"{"points":[]}"#.to_vec()),
        })
        .expect("the outbound request");
    let header = request.headers.get("api-key").expect("the api-key header");
    assert_eq!(header.to_str().ok(), Some("secret-key"));
    assert!(header.is_sensitive());
    assert_eq!(
        request.url.as_str(),
        "http://qdrant.test/collections/semantic-cache/points"
    );
    assert_eq!(request.query, vec![("wait".to_owned(), "true".to_owned())]);
}

#[test]
fn live_transport_adds_no_api_key_header_without_a_key() {
    let transport = LiveQdrantTransport::new(client(), "http://qdrant.test".to_owned(), None)
        .expect("the transport");
    let request = transport
        .build_request(search_request())
        .expect("the outbound request");
    assert!(request.headers.get("api-key").is_none());
}

#[test]
fn live_transport_rejects_an_unparsable_base() {
    let transport = LiveQdrantTransport::new(client(), String::new(), None).expect("the transport");
    let Err(error) = transport.build_request(search_request()) else {
        panic!("a path without a base must not build a URL");
    };
    assert!(error.to_string().contains("/collections/semantic-cache"));
}

#[test]
fn live_transport_rejects_a_key_a_header_cannot_carry() {
    let result = LiveQdrantTransport::new(
        client(),
        "http://qdrant.test".to_owned(),
        Some(SecretString::from("bad\nkey")),
    );
    let Err(error) = result else {
        panic!("a key with a line break must not reach a header");
    };
    assert_eq!(
        error.to_string(),
        "the Qdrant api key cannot be carried in an api-key header"
    );
}
