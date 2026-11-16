#![cfg(test)]

use super::support::{Harness, paths, settings, settings_with_account_key};
use crate::api::file_storage_contracts::storage::FileStorageBackend;

const SUPPLIED: &str = "https://foreign.blob.example/other/p/a.txt?sig=ignored";
const BLOB: &str = "https://acct.blob.core.windows.net/files/p/a.txt";
const DFS: &str = "https://acct.dfs.core.windows.net/files";

#[tokio::test]
async fn azure_download_addressing() {
    let harness = Harness::new(settings(), &[(200, "abc")]);
    let content = harness
        .backend
        .download_file(SUPPLIED)
        .await
        .expect("download");
    assert_eq!(content, b"abc");
    let calls = harness.calls();
    assert_eq!(paths(&calls), vec![("GET".to_owned(), BLOB.to_owned())]);
    let request = calls.first().expect("the download request");
    assert_eq!(request.header("x-ms-version"), Some("2019-07-07"));
    assert_eq!(request.header("authorization"), Some("Bearer tok-1"));
    assert_eq!(request.header("content-type"), None);
}

#[tokio::test]
async fn azure_download_invalid_urls() {
    for (supplied, expected) in [
        (
            "https://foreign.example/other/p/a.txt",
            "Invalid Azure Blob Storage URL: https://foreign.example/other/p/a.txt",
        ),
        (
            "https://foreign.blob.example/container",
            "Invalid Azure Blob Storage URL format: https://foreign.blob.example/container",
        ),
        (
            "https://foreign.blob.example/container/",
            "Invalid Azure Blob Storage URL format: https://foreign.blob.example/container/",
        ),
        ("not a url", "Invalid Azure Blob Storage URL: not a url"),
    ] {
        let harness = Harness::new(settings(), &[]);
        let error = harness
            .backend
            .download_file(supplied)
            .await
            .expect_err("an unusable URL");
        assert_eq!(error.to_string(), expected);
        assert!(harness.calls().is_empty(), "{supplied}");
    }
}

#[tokio::test]
async fn account_key_download() {
    let harness = Harness::new(settings_with_account_key(), &[(200, ""), (200, "abc")]);
    let content = harness
        .backend
        .download_file(SUPPLIED)
        .await
        .expect("download");
    assert_eq!(content, b"abc");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            ("GET".to_owned(), format!("{DFS}?restype=container")),
            ("GET".to_owned(), format!("{DFS}/p/a.txt")),
        ]
    );
}

#[tokio::test]
async fn account_key_download_missing_filesystem() {
    let harness = Harness::new(settings_with_account_key(), &[(404, "")]);
    let error = harness
        .backend
        .download_file(SUPPLIED)
        .await
        .expect_err("a missing filesystem");
    assert_eq!(error.to_string(), "Filesystem files does not exist");
    assert_eq!(harness.calls().len(), 1);
}

#[tokio::test]
async fn azure_download_propagates_status() {
    let token = Harness::new(settings(), &[(403, "")]);
    let error = token
        .backend
        .download_file(SUPPLIED)
        .await
        .expect_err("a forbidden download");
    assert!(error.to_string().contains("403"), "{error}");
    assert_eq!(token.calls().len(), 1);

    let account = Harness::new(settings_with_account_key(), &[(200, ""), (404, "")]);
    let error = account
        .backend
        .download_file(SUPPLIED)
        .await
        .expect_err("a missing file");
    assert!(error.to_string().contains("404"), "{error}");
    assert_eq!(account.calls().len(), 2);
}
