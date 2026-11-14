#![cfg(test)]

use std::sync::atomic::Ordering;

use super::support::{ACCOUNT, Reply, fixture, payload};
use crate::azure_storage_logging::logger::SERVICE_CLIENT_TTL;

const DIRECTORY_URL: &str = "https://acct.dfs.core.windows.net/logs/2023-11-14";
const EXISTS_URL: &str = "https://acct.blob.core.windows.net/logs/2023-11-14";

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
    let create_directory = calls.get(1).expect("the directory create");
    assert_eq!(create_directory.method, "PUT");
    assert_eq!(
        create_directory.url,
        format!("{DIRECTORY_URL}?resource=directory")
    );
    let create_file = calls.get(2).expect("the file create");
    assert_eq!(
        create_file.url,
        format!("{DIRECTORY_URL}/id-1.json?resource=file")
    );
    let append = calls.get(3).expect("the append");
    assert_eq!(
        append.url,
        format!("{DIRECTORY_URL}/id-1.json?action=append&position=0")
    );
    assert_eq!(append.header("x-ms-version"), Some("2019-07-07"));
    let authorization = append.header("Authorization").expect("signed");
    assert!(authorization.starts_with("SharedKey acct:"));
    assert!(!append.body.ends_with(b"\n"));
    let flush = calls.get(4).expect("the flush");
    assert_eq!(
        flush.url,
        format!(
            "{DIRECTORY_URL}/id-1.json?action=flush&position={}&retainUncommittedData=false&close=false",
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
