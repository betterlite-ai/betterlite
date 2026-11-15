#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::decisions::Json;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;

use super::{GcsTransportError, PooledGcsClient};
use crate::gcs_logger::tests::support::{Upstream, upstream};
use crate::gcs_mock::client::GcsHttpClient as _;
use crate::gcs_mock::request::{GcsReadRequest, GcsRequest, GcsWriteRequest};

fn client() -> PooledGcsClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    PooledGcsClient {
        client: ProxiedClient::outbound(&policy).expect("the HTTP client builds"),
    }
}

fn read_request(url: String, headers: HeaderMap) -> GcsRequest {
    GcsRequest::Get(GcsReadRequest {
        url,
        params: BTreeMap::from([("alt".to_owned(), "media".to_owned())]),
        headers,
        follow_redirects: None,
    })
}

fn write_request(server: &Upstream, headers: HeaderMap) -> GcsWriteRequest {
    GcsWriteRequest {
        url: format!("http://{}/upload/b/bucket/o", server.address),
        data: None,
        json: None,
        params: BTreeMap::from([("name".to_owned(), "2024-03-09/object".to_owned())]),
        headers,
        timeout: None,
        stream: false,
        content: None,
    }
}

#[tokio::test]
async fn a_get_carries_its_query_and_headers_and_returns_the_reply() {
    let server = upstream(200, "stored".to_owned()).await;
    let url = format!("http://{}/storage/v1/b/bucket/o/name", server.address);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer token-1"));
    let response = client()
        .send(read_request(url.clone(), headers))
        .await
        .expect("the read succeeds");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"stored");
    assert_eq!(response.url, url);
    assert_eq!(response.headers["x-reply"], "marker");
    assert!(response.elapsed > Duration::ZERO);
    let captured = server.captured();
    assert_eq!(captured.len(), 1);
    assert!(
        captured[0]
            .head
            .starts_with("GET /storage/v1/b/bucket/o/name?alt=media HTTP/1.1")
    );
    assert!(
        captured[0]
            .head
            .to_lowercase()
            .contains("authorization: bearer token-1")
    );
}

#[tokio::test]
async fn a_status_outside_2xx_is_returned_not_raised() {
    let server = upstream(404, "missing".to_owned()).await;
    let url = format!("http://{}/storage/v1/b/bucket/o/name", server.address);
    let response = client()
        .send(read_request(url, HeaderMap::new()))
        .await
        .expect("the reply arrives");
    assert_eq!(response.status, 404);
    assert_eq!(response.body, b"missing");
}

#[tokio::test]
async fn a_post_with_content_sends_the_bytes_under_the_given_content_type() {
    let server = upstream(200, "{}".to_owned()).await;
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    let mut request = write_request(&server, headers);
    request.content = Some(b"line-one\nline-two".to_vec());
    let response = client()
        .send(GcsRequest::Post(request))
        .await
        .expect("the upload succeeds");
    assert_eq!(response.status, 200);
    let captured = server.captured();
    assert!(
        captured[0]
            .head
            .starts_with("POST /upload/b/bucket/o?name=2024-03-09%2Fobject HTTP/1.1")
    );
    assert!(
        captured[0]
            .head
            .to_lowercase()
            .contains("content-type: application/x-ndjson")
    );
    assert_eq!(captured[0].body, "line-one\nline-two");
}

#[rstest]
#[case::raw_content(Some(b"{}".to_vec()), None, None, "{}")]
#[case::json(
    None,
    Some(Json::Object(IndexMap::from([("id".to_owned(), Json::Text("chatcmpl-1".to_owned()))]))),
    None,
    r#"{"id":"chatcmpl-1"}"#
)]
#[case::text_data(None, None, Some(Json::Text("{\"a\":1}".to_owned())), "{\"a\":1}")]
#[case::serialized_data(None, None, Some(Json::Bool(true)), "true")]
#[tokio::test]
async fn a_body_without_its_own_content_type_goes_out_as_json(
    #[case] content: Option<Vec<u8>>,
    #[case] json: Option<Json>,
    #[case] data: Option<Json>,
    #[case] expected: &str,
) {
    let server = upstream(200, "{}".to_owned()).await;
    let mut request = write_request(&server, HeaderMap::new());
    request.content = content;
    request.json = json;
    request.data = data;
    client()
        .send(GcsRequest::Post(request))
        .await
        .expect("the upload succeeds");
    let captured = server.captured();
    assert_eq!(captured[0].body, expected);
    assert!(
        captured[0]
            .head
            .to_lowercase()
            .contains("content-type: application/json")
    );
}

#[tokio::test]
async fn a_delete_without_a_body_sends_an_empty_one() {
    let server = upstream(204, String::new()).await;
    let response = client()
        .send(GcsRequest::Delete(write_request(&server, HeaderMap::new())))
        .await
        .expect("the delete succeeds");
    assert_eq!(response.status, 204);
    let captured = server.captured();
    assert!(captured[0].head.starts_with("DELETE /upload/b/bucket/o"));
    assert_eq!(captured[0].body, "");
}

#[tokio::test]
async fn a_malformed_url_is_refused_with_the_url_named() {
    let error = client()
        .send(read_request("not a url".to_owned(), HeaderMap::new()))
        .await
        .expect_err("the URL cannot be parsed");
    assert!(matches!(error, GcsTransportError::Url { url, .. } if url == "not a url"));
}

#[tokio::test]
async fn an_unreachable_host_is_a_send_error() {
    let error = client()
        .send(read_request(
            "http://127.0.0.1:1/storage".to_owned(),
            HeaderMap::new(),
        ))
        .await
        .expect_err("nothing listens on the port");
    assert!(matches!(error, GcsTransportError::Send(_)));
}
