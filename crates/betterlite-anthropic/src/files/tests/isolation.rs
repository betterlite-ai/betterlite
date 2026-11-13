use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_prompts::media_metadata::{FileBody, FileData};
use http::header::{HeaderMap, HeaderValue};

use super::support::{Fixture, NOW, TIMEOUT, client, configured, content, secret, upload};
use crate::files::error::FilesError;
use crate::files::request::{FileCall, FilesClient, Upload};

fn file() -> FileData {
    FileData::Direct(FileBody::Bytes(b"x".to_vec()))
}

fn identity(key: &str, base: &str) -> FilesClient {
    FilesClient {
        api_key: Some(secret(key)),
        api_base: Some(base.to_owned()),
        settings: configured(None, None, None),
        timeout: TIMEOUT,
    }
}

fn upload_call<'call>(
    file: &'call FileData,
    headers: &'call HeaderMap,
    boundary: &str,
) -> Upload<'call> {
    Upload {
        file: Some(file),
        purpose: None,
        boundary: boundary.to_owned(),
        headers,
    }
}

fn create(
    files_client: &FilesClient,
    file: &FileData,
    headers: &HeaderMap,
    boundary: &str,
) -> OutboundRequest {
    files_client
        .create_request(&upload_call(file, headers, boundary), NOW)
        .unwrap()
}

#[test]
fn files_credential_identity_isolation() {
    distinct_identities();
    caller_headers_stay_local();
}

fn distinct_identities() {
    let empty = HeaderMap::new();
    let file = file();
    let first = identity("sk-first-identity", "https://first.example");
    let second = identity("sk-second-identity", "https://second.example");
    let first_request = create(&first, &file, &empty, "first");
    let second_request = create(&second, &file, &empty, "second");
    assert_eq!(first_request.headers["x-api-key"], "sk-first-identity");
    assert_eq!(second_request.headers["x-api-key"], "sk-second-identity");
    assert_ne!(
        first_request.headers["x-api-key"],
        second_request.headers["x-api-key"]
    );
    assert_eq!(first_request.url.as_str(), "https://first.example/v1/files");
    assert_eq!(
        second_request.url.as_str(),
        "https://second.example/v1/files"
    );
}

fn caller_headers_stay_local() {
    let first = identity("sk-first-identity", "https://first.example");
    let file = file();
    let mut caller = HeaderMap::new();
    caller.insert("x-shared", HeaderValue::from_static("caller"));
    let with_caller = create(&first, &file, &caller, "first");
    assert_eq!(with_caller.headers["x-shared"], "caller");
    assert_eq!(caller.len(), 1);
    let plain = create(&first, &file, &HeaderMap::new(), "third");
    assert_eq!(plain.headers["x-api-key"], "sk-first-identity");
    assert!(!plain.headers.contains_key("x-shared"));
    assert!(matches!(plain.body, RequestBody::Multipart { .. }));
}

#[tokio::test]
async fn files_secret_safe_failure() {
    let key = "sk-secret-fixture-key";
    let token = "secret-fixture-token";
    let settings = configured(Some(key), Some(token), Some("https://secret.example"));
    assert!(!format!("{settings:?}").contains(key));
    assert!(!format!("{settings:?}").contains(token));
    let keyed = identity(key, "https://secret.example");
    assert!(!format!("{keyed:?}").contains(key));
    let file = file();
    let request = create(&keyed, &file, &HeaderMap::new(), "b");
    assert!(!format!("{request:?}").contains(key));
    failure_paths(&keyed, key, &file).await;
    invalid_header();
    missing_key();
}

async fn failure_paths(keyed: &FilesClient, key: &str, file: &FileData) {
    let failing = Fixture::failing();
    let headers = HeaderMap::new();
    let call = upload_call(file, &headers, "b");
    let refused = upload(&failing, keyed, &call).await.unwrap_err();
    assert!(matches!(refused, FilesError::Transport { .. }));
    assert!(!format!("{refused:?}").contains(key));
    assert!(!refused.to_string().contains(key));

    let denied = Fixture::answering(500, b"denied");
    let content_call = FileCall {
        file_id: "file-1",
        headers: &HeaderMap::new(),
    };
    let rejected = content(&denied, keyed, &content_call).await.unwrap_err();
    assert!(matches!(rejected, FilesError::Provider(_)));
    assert!(!format!("{rejected:?}").contains(key));
    assert!(!rejected.to_string().contains(key));
}

fn invalid_header() {
    let invalid = FilesClient {
        api_key: Some(secret("bad\nkey")),
        api_base: None,
        settings: configured(None, None, None),
        timeout: TIMEOUT,
    };
    let file = file();
    let error = invalid
        .create_request(&upload_call(&file, &HeaderMap::new(), "b"), NOW)
        .unwrap_err();
    assert!(matches!(error, FilesError::InvalidHeader { .. }));
    assert!(!error.to_string().contains("bad"));
}

fn missing_key() {
    let unset = client(None, None);
    let file = file();
    let error = unset
        .create_request(&upload_call(&file, &HeaderMap::new(), "b"), NOW)
        .unwrap_err();
    assert!(matches!(error, FilesError::MissingApiKey));
}
