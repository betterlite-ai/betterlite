#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use betterlite_core::rag::vertex_ingestion::ports::{
    GetRequest, JsonRequest, MultipartRequest, VertexRagHttp as _,
};
use betterlite_test_ports::refused_port::RefusedPort;
use secrecy::SecretString;

use super::VertexRagHttp;
use super::multipart_boundary;
use crate::rag_store_ports::test_support::{Reply, client, with_server};

const TOKEN: &str = "secret-token";

fn port() -> VertexRagHttp {
    VertexRagHttp {
        client: Arc::new(client()),
    }
}

fn token() -> SecretString {
    SecretString::from(TOKEN)
}

#[tokio::test]
async fn post_json_sends_the_bearer_and_the_body() {
    let answer = Reply::json(r#"{"name":"projects/p/locations/l/ragCorpora/1"}"#);
    let (result, seen) = with_server(answer, |base| async move {
        let token = token();
        port()
            .post_json(JsonRequest {
                url: &format!("{base}/v1beta1/projects/p/locations/l/ragCorpora"),
                token: &token,
                body: br#"{"displayName":"docs"}"#,
                timeout: Duration::from_secs(60),
            })
            .await
    })
    .await;
    let reply = result.expect("the corpus is created");
    assert_eq!(reply.status, 200);
    assert_eq!(
        reply.body.as_slice(),
        br#"{"name":"projects/p/locations/l/ragCorpora/1"}"#
    );
    let request = seen.first().expect("the create reaches the server");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1beta1/projects/p/locations/l/ragCorpora");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer secret-token")
    );
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(request.body, r#"{"displayName":"docs"}"#);
}

#[tokio::test]
async fn get_sends_the_bearer_without_a_body() {
    let answer = Reply::json(r#"{"name":"operations/1","done":true}"#);
    let (result, seen) = with_server(answer, |base| async move {
        let token = token();
        port()
            .get(GetRequest {
                url: &format!("{base}/v1beta1/projects/p/locations/l/operations/1"),
                token: &token,
                timeout: Duration::from_secs(60),
            })
            .await
    })
    .await;
    let reply = result.expect("the operation is read");
    assert_eq!(reply.status, 200);
    let request = seen.first().expect("the poll reaches the server");
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer secret-token")
    );
    assert!(!request.headers.contains_key("content-type"));
    assert!(request.body.is_empty());
}

#[tokio::test]
async fn post_multipart_sends_the_metadata_and_the_file() {
    let answer = Reply::json(r#"{"ragFile":{"name":"ragFiles/1"}}"#);
    let (result, seen) = with_server(answer, |base| async move {
        let token = token();
        port()
            .post_multipart(MultipartRequest {
                url: &format!("{base}/upload/v1beta1/projects/p/ragCorpora/1/ragFiles:upload"),
                token: &token,
                metadata: br#"{"rag_file":{"display_name":"notes.txt"}}"#,
                filename: "notes.txt",
                file: b"hello",
                file_content_type: "text/plain",
                timeout: Duration::from_secs(300),
            })
            .await
    })
    .await;
    let reply = result.expect("the file is uploaded");
    assert_eq!(reply.status, 200);
    let request = seen.first().expect("the upload reaches the server");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request
            .headers
            .get("x-goog-upload-protocol")
            .map(String::as_str),
        Some("multipart")
    );
    let content_type = request
        .headers
        .get("content-type")
        .expect("the multipart content type");
    assert!(content_type.starts_with("multipart/form-data; boundary=betterlite-"));
    assert!(request.body.contains(
        "Content-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{\"rag_file\":{\"display_name\":\"notes.txt\"}}\r\n"
    ));
    assert!(request.body.contains(
        "Content-Disposition: form-data; name=\"file\"; filename=\"notes.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n"
    ));
}

#[test]
fn the_multipart_boundary_is_the_parts_digest() {
    // sha256 of the metadata and then the file, so the boundary cannot repeat inside them
    assert_eq!(
        multipart_boundary(br#"{"rag_file":{"display_name":"notes.txt"}}"#, b"hello"),
        "betterlite-486380f0ed1bbecc5989a8d5376d835079c599eada654533c6172055009da8d9"
    );
}

#[tokio::test]
async fn replies_keep_a_non_2xx_status() {
    let answer = Reply::json(r#"{"error":{"message":"missing"}}"#).with_status(404);
    let (result, _seen) = with_server(answer, |base| async move {
        let token = token();
        port()
            .get(GetRequest {
                url: &format!("{base}/v1beta1/operations/missing"),
                token: &token,
                timeout: Duration::from_secs(60),
            })
            .await
    })
    .await;
    let reply = result.expect("a status is still a reply");
    assert_eq!(reply.status, 404);
}

#[tokio::test]
async fn requests_report_a_transport_failure_without_the_token() {
    let refused = RefusedPort::bind().expect("the refused port binds");
    let token = token();
    let error = port()
        .get(GetRequest {
            url: &format!("http://{}/v1beta1/operations/1", refused.address),
            token: &token,
            timeout: Duration::from_secs(1),
        })
        .await
        .err()
        .expect("nothing listens on the refused port");
    assert!(!error.to_string().contains(TOKEN));
    assert!(!format!("{error:?}").contains(TOKEN));
}

#[tokio::test]
async fn the_port_debug_names_the_type_only() {
    assert_eq!(format!("{:?}", port()), "VertexRagHttp { .. }");
}
