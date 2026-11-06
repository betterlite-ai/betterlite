#![cfg(test)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use super::{FILE_LIMIT, FileAccess as _, LocalFiles};

#[test]
fn local_files_write_then_read_and_truncate() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = LocalFiles;
    files.ensure_dir(directory.path()).unwrap();
    let path = directory.path().join("access-token");

    files.write(&path, b"first-token").unwrap();
    assert_eq!(files.read(&path).unwrap(), b"first-token");
    files.write(&path, b"new").unwrap();
    assert_eq!(files.read(&path).unwrap(), b"new");

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

#[test]
fn local_files_reject_oversized_or_missing_token_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("access-token");
    let mut files = LocalFiles;

    fs::write(&path, vec![b'x'; FILE_LIMIT]).unwrap();
    assert_eq!(files.read(&path).unwrap().len(), FILE_LIMIT);
    fs::write(&path, vec![b'x'; FILE_LIMIT + 1]).unwrap();
    files.read(&path).unwrap_err();
    files.read(&directory.path().join("missing")).unwrap_err();
}

#[test]
fn local_files_report_a_write_into_a_missing_directory() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = LocalFiles;

    assert!(
        files
            .write(&directory.path().join("missing/access-token"), b"x")
            .is_err()
    );
}
