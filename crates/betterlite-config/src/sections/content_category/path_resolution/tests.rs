#![cfg(test)]

use std::{fs, slice};

use super::paths::{CategoryPathError, resolve_category_file_path};

#[test]
fn path_resolution_jails_and_resolves_relative_paths() {
    super::tests::path_resolution_uses_root_and_rejects_escape();
}

#[test]
fn path_resolution_never_rebases_absolute_paths() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("category.yaml"), "keywords: []\n").unwrap();
    let outside = temp.path().join("category.yaml");
    let error =
        resolve_category_file_path(&outside, slice::from_ref(&root), &root, false).unwrap_err();
    assert!(matches!(error, CategoryPathError::OutsideRoots { .. }));
    let allowed =
        resolve_category_file_path(&outside, slice::from_ref(&root), &root, true).unwrap();
    assert_eq!(allowed, outside);
}
