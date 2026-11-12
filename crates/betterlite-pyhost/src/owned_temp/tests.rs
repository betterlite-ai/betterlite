#![cfg(test)]

use super::{OwnedTempDir, TempError};

#[test]
fn a_temporary_folder_is_removed_on_drop() {
    let directory = OwnedTempDir::new().unwrap();
    let path = directory.path().to_path_buf();
    assert!(path.is_dir());
    drop(directory);
    assert!(!path.exists());
}

#[test]
fn the_temporary_folder_is_named_for_python_hosts() {
    let directory = OwnedTempDir::new().unwrap();
    let name = directory.path().file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("betterlite-py-"), "got: {name}");
}

#[test]
fn file_joins_a_plain_name_to_the_folder() {
    let directory = OwnedTempDir::new().unwrap();
    let file = directory.file("payload.json").unwrap();
    assert_eq!(file, directory.path().join("payload.json"));
}

#[test]
fn file_rejects_a_separator() {
    let directory = OwnedTempDir::new().unwrap();
    assert!(matches!(
        directory.file("nested/payload.json"),
        Err(TempError::BadName { .. })
    ));
}

#[test]
fn file_rejects_a_parent_reference() {
    let directory = OwnedTempDir::new().unwrap();
    assert!(matches!(
        directory.file(".."),
        Err(TempError::BadName { .. })
    ));
    assert!(matches!(
        directory.file("../escape.json"),
        Err(TempError::BadName { .. })
    ));
    assert!(matches!(
        directory.file("payload..json"),
        Err(TempError::BadName { .. })
    ));
}

#[test]
fn file_rejects_an_empty_name() {
    let directory = OwnedTempDir::new().unwrap();
    assert!(matches!(directory.file(""), Err(TempError::BadName { .. })));
}
