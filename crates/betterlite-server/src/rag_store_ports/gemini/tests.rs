#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_core::rag::gemini_ingestion::port::{
    GeminiTransport as _, TransportBody, TransportMethod, TransportRequest,
};
use betterlite_models::decisions::Json;
use betterlite_test_ports::refused_port::RefusedPort;
use http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};

use super::GeminiHttp;
use crate::rag_store_ports::test_support::{Reply, client, with_server};

const API_KEY: &str = "gemini-key";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture text is JSON")
}

fn json_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("x-goog-api-key"),
        HeaderValue::from_static(API_KEY),
    );
    headers
}

fn put_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("content-length"),
        HeaderValue::from_static("5"),
    );
    headers.insert(
        HeaderName::from_static("x-goog-upload-offset"),
        HeaderValue::from_static("0"),
    );
    headers.insert(
        HeaderName::from_static("x-goog-upload-command"),
        HeaderValue::from_static("upload, finalize"),
    );
    headers
}

fn port() -> GeminiHttp {
    GeminiHttp {
        client: Arc::new(client()),
    }
}

#[tokio::test]
async fn send_posts_json_with_its_headers() {
    let answer = Reply::json(r#"{"name":"fileSearchStores/abc"}"#);
    let (result, seen) = with_server(answer, |base| async move {
        port()
            .send(TransportRequest {
                method: TransportMethod::Post,
                url: format!("{base}/fileSearchStores"),
                headers: json_headers(),
                body: TransportBody::Json(json(r#"{"displayName":"docs"}"#)),
                timeout: Duration::from_secs(60),
            })
            .await
    })
    .await;
    let reply = result.expect("the store is created");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body.as_slice(), br#"{"name":"fileSearchStores/abc"}"#);
    let request = seen.first().expect("the create reaches the server");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/fileSearchStores");
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        request.headers.get("x-goog-api-key").map(String::as_str),
        Some(API_KEY)
    );
    assert_eq!(request.body, r#"{"displayName":"docs"}"#);
}

#[tokio::test]
async fn send_puts_the_file_bytes() {
    let answer = Reply::json(r#"{"name":"files/xyz"}"#);
    let (result, seen) = with_server(answer, |base| async move {
        port()
            .send(TransportRequest {
                method: TransportMethod::Put,
                url: format!("{base}/upload/v1beta/fileSearchStores/abc:uploadToFileSearchStore"),
                headers: put_headers(),
                body: TransportBody::Bytes(b"hello".to_vec()),
                timeout: Duration::from_secs(300),
            })
            .await
    })
    .await;
    let reply = result.expect("the file is finalized");
    assert_eq!(reply.status, 200);
    let request = seen.first().expect("the finalize reaches the server");
    assert_eq!(request.method, "PUT");
    assert_eq!(request.body, "hello");
    assert_eq!(
        request.headers.get("content-length").map(String::as_str),
        Some("5")
    );
    assert_eq!(
        request
            .headers
            .get("x-goog-upload-command")
            .map(String::as_str),
        Some("upload, finalize")
    );
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/octet-stream")
    );
}

#[tokio::test]
async fn send_returns_a_non_2xx_reply_unchanged() {
    let answer = Reply::json(r#"{"error":{"message":"boom"}}"#).with_status(500);
    let (result, seen) = with_server(answer, |base| async move {
        port()
            .send(TransportRequest {
                method: TransportMethod::Post,
                url: format!("{base}/fileSearchStores"),
                headers: json_headers(),
                body: TransportBody::Json(json(r#"{"displayName":"docs"}"#)),
                timeout: Duration::from_secs(60),
            })
            .await
    })
    .await;
    let reply = result.expect("a status is still a reply");
    assert_eq!(reply.status, 500);
    assert_eq!(reply.body.as_slice(), br#"{"error":{"message":"boom"}}"#);
    assert_eq!(seen.len(), 1);
}

#[tokio::test]
async fn send_reports_a_transport_failure_without_the_key() {
    let refused = RefusedPort::bind().expect("the refused port binds");
    let error = port()
        .send(TransportRequest {
            method: TransportMethod::Post,
            url: format!("http://{}/fileSearchStores", refused.address),
            headers: json_headers(),
            body: TransportBody::Json(json(r#"{"displayName":"docs"}"#)),
            timeout: Duration::from_secs(1),
        })
        .await
        .expect_err("nothing listens on the refused port");
    assert!(!error.to_string().contains(API_KEY));
    assert!(!format!("{error:?}").contains(API_KEY));
}

#[tokio::test]
async fn send_reports_a_url_that_is_not_a_url() {
    let error = port()
        .send(TransportRequest {
            method: TransportMethod::Post,
            url: "not a url".to_owned(),
            headers: json_headers(),
            body: TransportBody::Json(json("{}")),
            timeout: Duration::from_secs(1),
        })
        .await
        .expect_err("the URL cannot be parsed");
    assert!(error.to_string().contains("is not a URL"));
}

#[tokio::test]
async fn send_reports_a_reply_over_the_client_body_cap() {
    let answer = Reply::json("0123456789abcdef");
    let (result, _seen) = with_server(answer, |base| async move {
        let mut capped = client();
        capped.limits.body_bytes = 8;
        GeminiHttp {
            client: Arc::new(capped),
        }
        .send(TransportRequest {
            method: TransportMethod::Post,
            url: format!("{base}/fileSearchStores"),
            headers: json_headers(),
            body: TransportBody::Json(json("{}")),
            timeout: Duration::from_secs(60),
        })
        .await
    })
    .await;
    let error = result.expect_err("the reply over the cap is a transport error");
    assert!(error.to_string().contains("larger than 8 bytes"));
}

#[tokio::test]
async fn the_port_debug_names_the_type_only() {
    assert_eq!(format!("{:?}", port()), "GeminiHttp { .. }");
}
