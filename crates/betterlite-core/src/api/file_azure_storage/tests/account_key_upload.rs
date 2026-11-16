#![cfg(test)]

use super::support::{Harness, paths, settings_with_account_key};
use crate::api::file_storage_contracts::storage::{
    FileNamingStrategy, FileStorageBackend, UploadOptions,
};

fn options(prefix: Option<&str>) -> UploadOptions<'_> {
    UploadOptions {
        path_prefix: prefix,
        file_naming_strategy: FileNamingStrategy::OriginalFilename,
    }
}

fn call(method: &str, url: &str) -> (String, String) {
    (method.to_owned(), url.to_owned())
}

const DFS: &str = "https://acct.dfs.core.windows.net/files";

#[tokio::test]
async fn account_key_upload() {
    let harness = Harness::new(
        settings_with_account_key(),
        &[
            (404, ""),
            (201, ""),
            (404, ""),
            (201, ""),
            (201, ""),
            (202, ""),
            (200, ""),
        ],
    );
    let url = harness
        .backend
        .upload_file(b"abc", "a.txt", "text/plain", options(Some("/p/")))
        .await
        .expect("upload");
    assert_eq!(url, "https://acct.blob.core.windows.net/files/p/a.txt");
    let calls = harness.calls();
    assert_eq!(
        paths(&calls),
        vec![
            call("GET", &format!("{DFS}?restype=container")),
            call("PUT", &format!("{DFS}?restype=container")),
            call("HEAD", &format!("{DFS}/p")),
            call("PUT", &format!("{DFS}/p?resource=directory")),
            call("PUT", &format!("{DFS}/p/a.txt?resource=file")),
            call("PATCH", &format!("{DFS}/p/a.txt?action=append&position=0")),
            call(
                "PATCH",
                &format!(
                    "{DFS}/p/a.txt?action=flush&position=3&retainUncommittedData=false&close=false"
                )
            ),
        ]
    );
    let append = calls.get(5).expect("the append request");
    assert_eq!(append.body, b"abc");
    assert_eq!(append.header("content-type"), Some("application/json"));
    assert_eq!(append.header("x-ms-version"), Some("2019-07-07"));
    assert!(
        append
            .header("authorization")
            .is_some_and(|value| value.starts_with("SharedKey acct:"))
    );
    let created = calls.get(4).expect("the create request");
    assert_eq!(created.body, b"");
    assert_eq!(created.header("content-type"), None);
    // the caller's content type never reaches Azure
    assert!(calls.iter().all(|request| {
        request
            .headers
            .iter()
            .all(|(_name, value)| !value.contains("text/plain"))
    }));
}

#[tokio::test]
async fn account_key_upload_skips_existing_resources() {
    let harness = Harness::new(
        settings_with_account_key(),
        &[(200, ""), (200, ""), (201, ""), (202, ""), (200, "")],
    );
    let url = harness
        .backend
        .upload_file(b"abc", "a.txt", "text/plain", options(Some("/p/")))
        .await
        .expect("upload");
    assert_eq!(url, "https://acct.blob.core.windows.net/files/p/a.txt");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            call("GET", &format!("{DFS}?restype=container")),
            call("HEAD", &format!("{DFS}/p")),
            call("PUT", &format!("{DFS}/p/a.txt?resource=file")),
            call("PATCH", &format!("{DFS}/p/a.txt?action=append&position=0")),
            call(
                "PATCH",
                &format!(
                    "{DFS}/p/a.txt?action=flush&position=3&retainUncommittedData=false&close=false"
                )
            ),
        ]
    );
}

#[tokio::test]
async fn account_key_upload_root_file() {
    let harness = Harness::new(
        settings_with_account_key(),
        &[(404, ""), (201, ""), (201, ""), (202, ""), (200, "")],
    );
    let url = harness
        .backend
        .upload_file(b"abc", "a.txt", "text/plain", options(None))
        .await
        .expect("upload");
    assert_eq!(url, "https://acct.blob.core.windows.net/files/a.txt");
    assert_eq!(
        paths(&harness.calls()),
        vec![
            call("GET", &format!("{DFS}?restype=container")),
            call("PUT", &format!("{DFS}?restype=container")),
            call("PUT", &format!("{DFS}/a.txt?resource=file")),
            call("PATCH", &format!("{DFS}/a.txt?action=append&position=0")),
            call(
                "PATCH",
                &format!(
                    "{DFS}/a.txt?action=flush&position=3&retainUncommittedData=false&close=false"
                )
            ),
        ]
    );
}

#[tokio::test]
async fn account_key_upload_rejects_an_empty_directory() {
    for prefix in ["/", ""] {
        let harness = Harness::new(settings_with_account_key(), &[(404, ""), (201, "")]);
        let error = harness
            .backend
            .upload_file(b"abc", "a.txt", "text/plain", options(Some(prefix)))
            .await
            .expect_err("an empty directory name");
        assert!(
            error.to_string().contains("empty directory name"),
            "{error}"
        );
        assert_eq!(harness.calls().len(), 2);
    }
}

#[tokio::test]
async fn account_key_upload_propagates_an_existence_error() {
    let harness = Harness::new(settings_with_account_key(), &[(403, "")]);
    let error = harness
        .backend
        .upload_file(b"abc", "a.txt", "text/plain", options(None))
        .await
        .expect_err("a forbidden existence check");
    assert!(error.to_string().contains("403"), "{error}");
    assert_eq!(harness.calls().len(), 1);
}
