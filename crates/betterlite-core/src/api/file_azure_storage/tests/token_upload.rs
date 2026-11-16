#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::api::file_azure_storage::Settings;

use super::support::{Harness, paths, settings};
use crate::api::file_storage_contracts::storage::{
    FileNamingStrategy, FileStorageBackend, UploadOptions,
};

const DFS: &str = "https://acct.dfs.core.windows.net/files";

fn original(prefix: Option<&str>) -> UploadOptions<'_> {
    UploadOptions {
        path_prefix: prefix,
        file_naming_strategy: FileNamingStrategy::OriginalFilename,
    }
}

#[tokio::test]
async fn token_upload_sequence() {
    let harness = Harness::new(settings(), &[(201, ""), (202, ""), (200, "")]);
    let url = harness
        .backend
        .upload_file(b"abc", "a b.TXT", "text/plain", original(Some("/p/")))
        .await
        .expect("upload");
    assert_eq!(url, "https://acct.blob.core.windows.net/files/p/a%20b.TXT");
    let calls = harness.calls();
    assert_eq!(
        paths(&calls),
        vec![
            ("PUT".to_owned(), format!("{DFS}/p/a%20b.TXT?resource=file")),
            (
                "PATCH".to_owned(),
                format!("{DFS}/p/a%20b.TXT?action=append&position=0")
            ),
            (
                "PATCH".to_owned(),
                format!("{DFS}/p/a%20b.TXT?action=flush&position=3")
            ),
        ]
    );
    let create = calls.first().expect("the create request");
    assert_eq!(create.body, b"");
    assert_eq!(create.header("content-length"), Some("0"));
    assert_eq!(create.header("content-type"), None);
    let append = calls.get(1).expect("the append request");
    assert_eq!(append.body, b"abc");
    assert_eq!(
        append.header("content-type"),
        Some("application/octet-stream")
    );
    assert_eq!(append.header("x-ms-version"), Some("2019-07-07"));
    assert_eq!(append.header("authorization"), Some("Bearer tok-1"));
    let flush = calls.get(2).expect("the flush request");
    assert_eq!(flush.body, b"");
    assert_eq!(flush.header("content-length"), Some("0"));
    assert_eq!(flush.header("content-type"), None);
}

// LiteLLM strips slashes off a prefix and then joins the empty result with a slash, so a
// slash-only prefix leaves one extra before the name in both the request and the returned URL
#[tokio::test]
async fn token_upload_prefixes() {
    for (prefix, expected) in [
        (Some("/"), "//a.txt"),
        (Some("///"), "//a.txt"),
        (Some(""), "//a.txt"),
        (None, "/a.txt"),
    ] {
        let harness = Harness::new(settings(), &[(201, ""), (202, ""), (200, "")]);
        let url = harness
            .backend
            .upload_file(b"abc", "a.txt", "text/plain", original(prefix))
            .await
            .expect("upload");
        assert_eq!(
            url,
            format!("https://acct.blob.core.windows.net/files{expected}"),
            "{prefix:?}"
        );
        assert_eq!(
            paths(&harness.calls())
                .first()
                .map(|(_method, url)| url.clone()),
            Some(format!("{DFS}{expected}?resource=file")),
            "{prefix:?}"
        );
    }
}

#[tokio::test]
async fn token_upload_failed_append() {
    for status in [403, 429, 500] {
        let harness = Harness::new(settings(), &[(201, ""), (status, "")]);
        let error = harness
            .backend
            .upload_file(b"abc", "a.txt", "text/plain", original(None))
            .await
            .expect_err("a failed append");
        assert!(error.to_string().contains(&status.to_string()), "{error}");
        let calls = harness.calls();
        assert_eq!(calls.len(), 2, "{status}");
        assert!(calls.iter().all(|call| call.method != "DELETE"), "{status}");
    }
}

#[tokio::test]
async fn account_key_precedence() {
    let harness = Harness::new(
        super::support::settings_with_account_key(),
        &[(200, ""), (201, ""), (202, ""), (200, "")],
    );
    let url = harness
        .backend
        .upload_file(b"abc", "a.txt", "text/plain", original(None))
        .await
        .expect("upload");
    assert_eq!(url, "https://acct.blob.core.windows.net/files/a.txt");
    let create = harness.calls().get(1).expect("the create request");
    assert!(
        create
            .header("authorization")
            .is_some_and(|value| value.starts_with("SharedKey acct:"))
    );
    assert!(
        harness
            .calls()
            .iter()
            .all(|call| call.header("authorization") != Some("Bearer tok-1"))
    );
}

// AZURE_STORAGE_ACCOUNT_KEY="" reads as unset, so the token branch handles the upload
#[tokio::test]
async fn empty_account_key_uses_token_auth() {
    let environment = BTreeMap::from([
        ("AZURE_STORAGE_ACCOUNT_NAME".to_owned(), "acct".to_owned()),
        ("AZURE_STORAGE_FILE_SYSTEM".to_owned(), "files".to_owned()),
        ("AZURE_STORAGE_ACCOUNT_KEY".to_owned(), String::new()),
    ]);
    let settings = Settings::from_environment(&environment).expect("the settings");
    assert!(settings.account_key.is_none());
    let harness = Harness::new(settings, &[(201, ""), (202, ""), (200, "")]);
    let url = harness
        .backend
        .upload_file(b"abc", "a.txt", "text/plain", original(None))
        .await
        .expect("upload");
    assert_eq!(url, "https://acct.blob.core.windows.net/files/a.txt");
    let calls = harness.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls.first().and_then(|call| call.header("authorization")),
        Some("Bearer tok-1")
    );
}
