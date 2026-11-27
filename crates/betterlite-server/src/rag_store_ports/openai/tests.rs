#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_core::api::vector_store_http_write::error::BaseLlmException;
use betterlite_core::rag::openai_ingestion::{
    AttachFile, CreateStore, ExpiresAfter, FileUploadPort as _, UploadFile, VectorFilePort as _,
    VectorStorePort as _,
};
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::FilesError;
use secrecy::SecretString;
use serde_json::value::RawValue;

use super::wire::openai_headers;
use super::{OpenAiFilePort, OpenAiStorePort, OpenAiUploadPort};
use crate::rag_store_ports::test_support::{Reply, client, with_server};

const CREATE_REPLY: &str = r#"{"id":"vs_123","object":"vector_store"}"#;
const FILE_REPLY: &str = r#"{"id":"file-1","bytes":5,"created_at":1700000000,"filename":"notes.txt","object":"file","purpose":"assistants"}"#;

fn key() -> SecretString {
    SecretString::from("sk-test")
}

fn store_port() -> OpenAiStorePort {
    OpenAiStorePort {
        client: Arc::new(client()),
    }
}

fn file_port() -> OpenAiFilePort {
    OpenAiFilePort {
        client: Arc::new(client()),
    }
}

fn upload_port() -> OpenAiUploadPort {
    OpenAiUploadPort {
        client: Arc::new(client()),
    }
}

#[tokio::test]
async fn the_ports_debug_with_the_type_only() {
    assert_eq!(format!("{:?}", store_port()), "OpenAiStorePort { .. }");
    assert_eq!(format!("{:?}", file_port()), "OpenAiFilePort { .. }");
    assert_eq!(format!("{:?}", upload_port()), "OpenAiUploadPort { .. }");
}

#[test]
fn the_authorization_header_is_marked_sensitive() {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_owned(), key());
    headers.insert(
        "openai-beta".to_owned(),
        SecretString::from("assistants=v2"),
    );
    let mapped = openai_headers(headers).expect("the headers map");
    let authorization = mapped
        .get("authorization")
        .expect("the authorization header");
    assert!(!format!("{authorization:?}").contains("sk-test"));
    let beta = mapped.get("openai-beta").expect("the beta header");
    assert!(format!("{beta:?}").contains("assistants=v2"));
}

#[tokio::test]
async fn create_store_posts_the_native_request_and_returns_the_id() {
    let (result, seen) = with_server(Reply::json(CREATE_REPLY), |base| async move {
        let key = key();
        store_port()
            .create_store(CreateStore {
                name: "docs",
                expires_after: Some(ExpiresAfter {
                    anchor: "last_active_at",
                    days: 7,
                }),
                api_key: Some(&key),
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    assert_eq!(
        result.expect("the store is created"),
        Some("vs_123".to_owned())
    );
    let request = seen.first().expect("the create reaches the server");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/vector_stores");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(
        request.headers.get("openai-beta").map(String::as_str),
        Some("assistants=v2")
    );
    let body = serde_json::from_str::<Json>(&request.body).expect("the body is JSON");
    let expected = serde_json::from_str::<Json>(
        r#"{"name":"docs","file_ids":null,"expires_after":{"anchor":"last_active_at","days":7},"chunking_strategy":null,"metadata":null}"#,
    )
    .expect("the expected body is JSON");
    assert_eq!(body, expected);
}

#[tokio::test]
async fn create_store_reports_a_missing_id_as_none() {
    let (result, seen) = with_server(
        Reply::json(r#"{"object":"vector_store"}"#),
        |base| async move {
            store_port()
                .create_store(CreateStore {
                    name: "docs",
                    expires_after: None,
                    api_key: None,
                    api_base: Some(base.as_str()),
                })
                .await
        },
    )
    .await;
    assert_eq!(result.expect("the reply is read"), None::<String>);
    assert_eq!(seen.len(), 1);
}

#[tokio::test]
async fn create_store_sends_bearer_none_without_a_key() {
    let (result, seen) = with_server(Reply::json(CREATE_REPLY), |base| async move {
        store_port()
            .create_store(CreateStore {
                name: "docs",
                expires_after: None,
                api_key: None,
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    assert_eq!(
        result.expect("the store is created"),
        Some("vs_123".to_owned())
    );
    let request = seen.first().expect("the create reaches the server");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer None")
    );
}

#[tokio::test]
async fn create_store_turns_a_status_error_into_a_port_error() {
    let body = r#"{"error":{"message":"bad request"}}"#;
    let (result, seen) = with_server(Reply::json(body).with_status(400), |base| async move {
        store_port()
            .create_store(CreateStore {
                name: "docs",
                expires_after: None,
                api_key: None,
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    let error = result.expect_err("the status is an error");
    assert_eq!(error.message, body);
    let native = error
        .source
        .downcast_ref::<BaseLlmException>()
        .expect("the native error is the source");
    assert_eq!(native.status_code, 400);
    assert_eq!(seen.len(), 1);
}

#[tokio::test]
async fn attach_posts_the_file_id_and_the_chunking_strategy() {
    let strategy =
        r#"{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}"#;
    let answer = Reply::json(r#"{"id":"file-abc","object":"vector_store.file"}"#);
    let (result, seen) = with_server(answer, |base| async move {
        let chunking: &RawValue = serde_json::from_str(strategy).expect("the strategy is JSON");
        let key = key();
        file_port()
            .attach(AttachFile {
                vector_store_id: "vs_1",
                file_id: "file-9",
                chunking_strategy: chunking,
                api_key: Some(&key),
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    result.expect("the file is attached");
    let request = seen.first().expect("the attach reaches the server");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/vector_stores/vs_1/files");
    let body = serde_json::from_str::<Json>(&request.body).expect("the body is JSON");
    let expected = serde_json::from_str::<Json>(
        r#"{"file_id":"file-9","chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}}}"#,
    )
    .expect("the expected body is JSON");
    assert_eq!(body, expected);
}

#[tokio::test]
async fn attach_turns_a_status_error_into_a_port_error() {
    let body = r#"{"error":{"message":"missing store"}}"#;
    let (result, _seen) = with_server(Reply::json(body).with_status(500), |base| async move {
        let chunking: &RawValue = serde_json::from_str("null").expect("null is JSON");
        file_port()
            .attach(AttachFile {
                vector_store_id: "vs_1",
                file_id: "file-9",
                chunking_strategy: chunking,
                api_key: None,
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    let error = result.expect_err("the status is an error");
    assert_eq!(error.message, body);
    assert!(
        error
            .source
            .downcast_ref::<BaseLlmException>()
            .is_some_and(|native| native.status_code == 500)
    );
}

#[tokio::test]
async fn upload_posts_the_file_with_the_assistants_purpose() {
    let (result, seen) = with_server(Reply::json(FILE_REPLY), |base| async move {
        let key = key();
        upload_port()
            .upload(UploadFile {
                filename: "notes.txt",
                content: b"hello",
                content_type: "text/plain",
                purpose: "assistants",
                api_key: Some(&key),
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    assert_eq!(result.expect("the file is uploaded"), "file-1");
    let request = seen.first().expect("the upload reaches the server");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/files");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    let content_type = request
        .headers
        .get("content-type")
        .expect("the multipart content type");
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    assert!(
        request
            .body
            .contains("Content-Disposition: form-data; name=\"purpose\"\r\n\r\nassistants\r\n")
    );
    assert!(request.body.contains(
        "Content-Disposition: form-data; name=\"file\"; filename=\"notes.txt\"\r\nContent-Type: text/plain\r\n\r\nhello\r\n"
    ));
}

#[tokio::test]
async fn upload_reports_an_unknown_purpose() {
    let (result, seen) = with_server(Reply::json(FILE_REPLY), |base| async move {
        upload_port()
            .upload(UploadFile {
                filename: "notes.txt",
                content: b"hello",
                content_type: "text/plain",
                purpose: "vision",
                api_key: None,
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    let error = result.expect_err("the purpose is refused");
    assert!(error.message.contains("vision"));
    assert!(seen.is_empty());
}

#[tokio::test]
async fn upload_reports_a_missing_key_as_not_initialized() {
    let (result, seen) = with_server(Reply::json(FILE_REPLY), |base| async move {
        upload_port()
            .upload(UploadFile {
                filename: "notes.txt",
                content: b"hello",
                content_type: "text/plain",
                purpose: "assistants",
                api_key: None,
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    let error = result.expect_err("the upload needs a key");
    assert!(error.message.contains("OpenAI client is not initialized"));
    let native = error
        .source
        .downcast_ref::<FilesError>()
        .expect("the native error is the source");
    assert!(matches!(native, FilesError::NotInitialized));
    assert!(seen.is_empty());
}

#[tokio::test]
async fn upload_turns_a_status_error_into_a_port_error() {
    let body = r#"{"error":{"message":"bad file"}}"#;
    let (result, _seen) = with_server(Reply::json(body).with_status(400), |base| async move {
        let key = key();
        upload_port()
            .upload(UploadFile {
                filename: "notes.txt",
                content: b"hello",
                content_type: "text/plain",
                purpose: "assistants",
                api_key: Some(&key),
                api_base: Some(base.as_str()),
            })
            .await
    })
    .await;
    let error = result.expect_err("the status is an error");
    assert_eq!(error.message, format!("Error code: 400 - {body}"));
    assert!(
        error
            .source
            .downcast_ref::<FilesError>()
            .is_some_and(|native| matches!(native, FilesError::Status(_)))
    );
}
