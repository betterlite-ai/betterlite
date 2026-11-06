#![cfg(test)]

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::AsyncWrite;

use super::{FILE_LIMIT, FileAccess as _, LocalFiles, write_and_flush};

#[tokio::test]
async fn local_files_write_then_read_and_truncate() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = LocalFiles;
    files.ensure_dir(directory.path()).unwrap();
    let path = directory.path().join("access-token");

    files.write(&path, b"first-token").await.unwrap();
    assert_eq!(files.read(&path).await.unwrap(), b"first-token");
    files.write(&path, b"new").await.unwrap();
    assert_eq!(files.read(&path).await.unwrap(), b"new");

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn local_files_create_a_missing_directory_and_skip_an_existing_one() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("tokens");
    let mut files = LocalFiles;

    files.ensure_dir(&nested).unwrap();
    assert!(nested.is_dir());
    files.ensure_dir(&nested).unwrap();
    assert!(files.ensure_dir(Path::new("")).is_err());
}

#[tokio::test]
async fn local_files_reject_oversized_or_missing_token_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("access-token");
    let mut files = LocalFiles;

    fs::write(&path, vec![b'x'; FILE_LIMIT]).unwrap();
    assert_eq!(files.read(&path).await.unwrap().len(), FILE_LIMIT);
    fs::write(&path, vec![b'x'; FILE_LIMIT + 1]).unwrap();
    files.read(&path).await.unwrap_err();
    files
        .read(&directory.path().join("missing"))
        .await
        .unwrap_err();
}

struct FailingFlush;

impl AsyncWrite for FailingFlush {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("flush failed")))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn a_flush_failure_is_reported_as_a_write_failure() {
    let mut writer = FailingFlush;

    write_and_flush(&mut writer, b"token").await.unwrap_err();
}

#[tokio::test]
async fn local_files_report_a_write_into_a_missing_directory() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = LocalFiles;

    assert!(
        files
            .write(&directory.path().join("missing/access-token"), b"x")
            .await
            .is_err()
    );
}
