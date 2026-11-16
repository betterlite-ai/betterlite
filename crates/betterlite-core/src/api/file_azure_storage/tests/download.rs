#![cfg(test)]

use rstest::rstest;

use super::support::{Harness, paths, settings, settings_with_account_key};
use crate::api::file_storage_contracts::storage::FileStorageBackend as _;

const SUPPLIED: &str = "https://foreign.blob.example/other/p/a.txt?sig=ignored";
const FILES: &str = "https://acct.blob.core.windows.net/files";
const BLOB: &str = "https://acct.blob.core.windows.net/files/p/a.txt";

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

#[rstest]
#[case::foreign_host(
    "https://foreign.example/other/p/a.txt",
    "Invalid Azure Blob Storage URL: https://foreign.example/other/p/a.txt"
)]
#[case::container_only(
    "https://foreign.blob.example/container",
    "Invalid Azure Blob Storage URL format: https://foreign.blob.example/container"
)]
#[case::container_with_slash(
    "https://foreign.blob.example/container/",
    "Invalid Azure Blob Storage URL format: https://foreign.blob.example/container/"
)]
#[case::not_a_url("not a url", "Invalid Azure Blob Storage URL: not a url")]
#[tokio::test]
async fn azure_download_invalid_urls(#[case] supplied: &str, #[case] expected: &str) {
    let harness = Harness::new(settings(), &[]);
    let error = harness
        .backend
        .download_file(supplied)
        .await
        .expect_err("an unusable URL");
    assert_eq!(error.to_string(), expected);
    assert!(harness.calls().is_empty(), "{supplied}");
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
            ("GET".to_owned(), format!("{FILES}?restype=container")),
            ("GET".to_owned(), BLOB.to_owned()),
        ]
    );
}

// the SDK's get_file_client strips leading and trailing / from the path it turns into a blob name
#[rstest]
#[case::trailing("https://acct.blob.core.windows.net/files/p/a.txt/")]
#[case::leading("https://acct.blob.core.windows.net/files//p/a.txt")]
#[case::both("https://acct.blob.core.windows.net/files//p/a.txt//")]
#[tokio::test]
async fn account_key_download_strips_the_blob_path(#[case] supplied: &str) {
    let harness = Harness::new(settings_with_account_key(), &[(200, ""), (200, "abc")]);
    let content = harness
        .backend
        .download_file(supplied)
        .await
        .expect("download");
    assert_eq!(content, b"abc");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            ("GET".to_owned(), format!("{FILES}?restype=container")),
            ("GET".to_owned(), BLOB.to_owned()),
        ]
    );
}

// get_file_client raises ValueError for a path of only slashes, after the filesystem check
#[tokio::test]
async fn account_key_download_rejects_a_slash_only_path() {
    let harness = Harness::new(settings_with_account_key(), &[(200, "")]);
    let supplied = "https://acct.blob.core.windows.net/files///";
    let error = harness
        .backend
        .download_file(supplied)
        .await
        .expect_err("a path of only slashes");
    assert_eq!(
        error.to_string(),
        format!("Invalid Azure Blob Storage URL format: {supplied}")
    );
    assert_eq!(harness.calls().len(), 1);
}

// get_file_client keeps a lone / as the blob name instead of stripping it to nothing
#[tokio::test]
async fn account_key_download_keeps_a_lone_slash() {
    let harness = Harness::new(settings_with_account_key(), &[(200, ""), (404, "")]);
    let error = harness
        .backend
        .download_file("https://acct.blob.core.windows.net/files//")
        .await
        .expect_err("the root blob");
    assert!(error.to_string().contains("404"), "{error}");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            ("GET".to_owned(), format!("{FILES}?restype=container")),
            ("GET".to_owned(), format!("{FILES}//")),
        ]
    );
}

// LiteLLM's token branch builds its URL from the path as parsed, with no SDK strip to apply
#[rstest]
#[case::trailing("https://acct.blob.core.windows.net/files/p/a.txt/")]
#[case::leading("https://acct.blob.core.windows.net/files//p/a.txt")]
#[tokio::test]
async fn token_download_keeps_the_raw_path(#[case] supplied: &str) {
    let harness = Harness::new(settings(), &[(200, "abc")]);
    let content = harness
        .backend
        .download_file(supplied)
        .await
        .expect("download");
    assert_eq!(content, b"abc");
    assert_eq!(
        paths(&harness.calls()),
        vec![("GET".to_owned(), supplied.to_owned())]
    );
}

// urlparse keeps a raw space and the SDK quotes it once, so the URL parser's encoding stays raw
#[tokio::test]
async fn account_key_download_quotes_the_caller_path() {
    let harness = Harness::new(settings_with_account_key(), &[(200, ""), (200, "abc")]);
    let content = harness
        .backend
        .download_file("https://acct.blob.core.windows.net/files/my dir/a.txt")
        .await
        .expect("download");
    assert_eq!(content, b"abc");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            ("GET".to_owned(), format!("{FILES}?restype=container")),
            ("GET".to_owned(), format!("{FILES}/my%20dir/a.txt")),
        ]
    );
}

// a name the upload already quoted is quoted once more, exactly as the SDK does for its URL
#[tokio::test]
async fn account_key_download_quotes_a_quoted_name() {
    let harness = Harness::new(settings_with_account_key(), &[(200, ""), (200, "abc")]);
    harness
        .backend
        .download_file("https://acct.blob.core.windows.net/files/p/a%20b.txt")
        .await
        .expect("download");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            ("GET".to_owned(), format!("{FILES}?restype=container")),
            ("GET".to_owned(), format!("{FILES}/p/a%2520b.txt")),
        ]
    );
}

// urlparse drops the last segment's ;params before LiteLLM hands the path to the SDK
#[rstest]
#[case::one_letter("https://acct.blob.core.windows.net/files/p/a;b.txt", "a")]
#[case::several_letters("https://acct.blob.core.windows.net/files/p/ab;cd.txt", "ab")]
#[tokio::test]
async fn account_key_download_drops_the_last_segment_params(
    #[case] supplied: &str,
    #[case] file_name: &str,
) {
    let harness = Harness::new(settings_with_account_key(), &[(200, ""), (200, "abc")]);
    harness
        .backend
        .download_file(supplied)
        .await
        .expect("download");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            ("GET".to_owned(), format!("{FILES}?restype=container")),
            ("GET".to_owned(), format!("{FILES}/p/{file_name}")),
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
    let missing = account
        .backend
        .download_file(SUPPLIED)
        .await
        .expect_err("a missing file");
    assert!(missing.to_string().contains("404"), "{missing}");
    assert_eq!(account.calls().len(), 2);
}

// httpx's raise_for_status rejects a redirect, so a 302 body never becomes the downloaded file
#[tokio::test]
async fn azure_download_rejects_a_redirect() {
    let harness = Harness::new(settings(), &[(302, "moved")]);
    let error = harness
        .backend
        .download_file(SUPPLIED)
        .await
        .expect_err("a redirected download");
    assert_eq!(
        error.to_string(),
        "the Azure storage service answered with status 302 for download"
    );
    assert_eq!(harness.calls().len(), 1);
}
