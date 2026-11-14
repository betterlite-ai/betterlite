#![cfg(test)]

use std::sync::atomic::Ordering;
use std::time::Duration;

use http::Method;
use secrecy::SecretString;

use super::support::{ACCOUNT, Reply, START_SECONDS, fixture, payload};
use crate::azure_storage_logging::http_ops::{StorageAuthorization, storage_request};
use crate::azure_storage_logging::logger::SERVICE_CLIENT_TTL;

const DIRECTORY_URL: &str = "https://acct.dfs.core.windows.net/logs/2023-11-14";
const EXISTS_URL: &str = "https://acct.blob.core.windows.net/logs/2023-11-14";
const FLUSH_QUERY: &str = "&retainUncommittedData=false&close=false";

#[tokio::test]
async fn the_account_key_path_creates_the_dated_directory_and_the_file() {
    let fixture = fixture(&ACCOUNT, &[Reply::NOT_FOUND]);
    let payload = payload("id-1");
    fixture
        .logger
        .upload_payload(&payload)
        .await
        .expect("the scripted account-key upload succeeds");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 5);
    let exists = calls.first().expect("the directory probe");
    assert_eq!(exists.method, "HEAD");
    assert_eq!(exists.url, EXISTS_URL);
    assert_eq!(exists.header("Content-Length"), None);
    let dir = calls.get(1).expect("the directory create");
    assert_eq!(dir.method, "PUT");
    assert_eq!(dir.url, format!("{DIRECTORY_URL}?resource=directory"));
    let file_url = format!("{DIRECTORY_URL}/id-1.json");
    let create_file = calls.get(2).expect("the file create");
    assert_eq!(create_file.url, format!("{file_url}?resource=file"));
    let append = calls.get(3).expect("the append");
    assert_eq!(append.url, format!("{file_url}?action=append&position=0"));
    assert_eq!(append.header("x-ms-version"), Some("2019-07-07"));
    let authorization = append.header("Authorization").expect("signed");
    assert!(authorization.starts_with("SharedKey acct:"));
    assert!(!append.body.ends_with(b"\n"));
    let flush = calls.get(4).expect("the flush");
    assert_eq!(
        flush.url,
        format!(
            "{file_url}?action=flush&position={}{FLUSH_QUERY}",
            append.body.len()
        )
    );
    assert_eq!(fixture.logger.created_clients.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.logger.closed_clients.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_existing_directory_is_not_created_again() {
    let fixture = fixture(&ACCOUNT, &[Reply::OK]);
    fixture
        .logger
        .upload_payload(&payload("id-1"))
        .await
        .expect("upload");
    let calls = fixture.transport.calls();
    assert_eq!(calls.len(), 4);
    assert_eq!(
        calls.get(1).expect("the file create").url,
        format!("{DIRECTORY_URL}/id-1.json?resource=file")
    );
}

#[tokio::test]
async fn the_client_is_closed_and_recreated_after_its_ttl() {
    let fixture = fixture(&ACCOUNT, &[Reply::OK]);
    fixture
        .logger
        .upload_payload(&payload("id-1"))
        .await
        .expect("first upload");
    fixture
        .clock
        .advance(SERVICE_CLIENT_TTL.as_secs().saturating_sub(1));
    fixture
        .logger
        .upload_payload(&payload("id-2"))
        .await
        .expect("second upload reuses the client");
    assert_eq!(fixture.logger.created_clients.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.logger.closed_clients.load(Ordering::SeqCst), 0);
    fixture.clock.advance(1);
    fixture
        .logger
        .upload_payload(&payload("id-3"))
        .await
        .expect("third upload rebuilds the client");
    assert_eq!(fixture.logger.created_clients.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.logger.closed_clients.load(Ordering::SeqCst), 1);
}

// the expected value is Python's hmac over the documented string to sign, with base64 "a2V5" as the key
#[test]
fn the_shared_key_signature_matches_an_independent_hmac() {
    let key = SecretString::from("a2V5");
    let auth = StorageAuthorization::SharedKey {
        account: "acct",
        key: &key,
    };
    let request = storage_request(
        Method::PATCH,
        format!("{DIRECTORY_URL}/id-1.json?action=append&position=0"),
        &auth,
        Duration::from_secs(START_SECONDS),
        b"{}",
    )
    .expect("the request signs");
    let expected = [
        ("x-ms-version", "2019-07-07"),
        ("Content-Type", "application/json"),
        ("x-ms-date", "Tue, 14 Nov 2023 22:13:20 GMT"),
        (
            "Authorization",
            "SharedKey acct:u5B7KqcJR/sfUZRolDqpee7V3TFE4DMZ05eL6RHNesc=",
        ),
    ]
    .map(|(name, value)| (name.to_owned(), value.to_owned()));
    assert_eq!(request.headers, expected);
}

#[test]
fn a_key_that_is_not_base64_does_not_sign() {
    let key = SecretString::from("not base64!");
    let auth = StorageAuthorization::SharedKey {
        account: "acct",
        key: &key,
    };
    let error = storage_request(
        Method::PATCH,
        format!("{DIRECTORY_URL}?action=append&position=0"),
        &auth,
        Duration::ZERO,
        b"{}",
    )
    .expect_err("the key must decode");
    assert!(error.to_string().contains("base64"));
}

#[tokio::test]
async fn a_failed_directory_probe_stops_the_upload() {
    let fixture = fixture(
        &ACCOUNT,
        &[Reply {
            status: 400,
            body: "",
        }],
    );
    let error = fixture
        .logger
        .upload_payload(&payload("id-1"))
        .await
        .expect_err("a 400 from exists() fails the upload");
    assert!(error.to_string().contains("400 for exists"));
    assert_eq!(fixture.transport.calls().len(), 1);
}

#[tokio::test]
async fn a_failed_directory_create_stops_the_upload() {
    let fixture = fixture(
        &ACCOUNT,
        &[
            Reply::NOT_FOUND,
            Reply {
                status: 403,
                body: "",
            },
        ],
    );
    let error = fixture
        .logger
        .upload_payload(&payload("id-1"))
        .await
        .expect_err("a 403 fails the upload");
    assert!(error.to_string().contains("403"));
    assert_eq!(fixture.transport.calls().len(), 2);
}
