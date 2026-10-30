#![cfg(test)]

use super::super::{ConfigMap, IncludeError, resolve_includes};
use serde_json::json;
use std::fs;
use tempfile::tempdir;

fn config(entries: &[&str]) -> ConfigMap {
    let mut config = ConfigMap::new();
    config.insert("include".to_owned(), json!(entries));
    config
}

#[test]
fn merges_lists_and_replaces_other_values() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    let included = directory.path().join("included.yaml");
    fs::write(
        &root,
        "include: [included.yaml]\nitems: [root]\nvalue: root\n",
    )
    .unwrap();
    fs::write(&included, "items: [included]\nvalue: included\n").unwrap();
    let mut root_config = config(&["included.yaml"]);
    root_config.insert("items".to_owned(), json!(["root"]));
    root_config.insert("value".to_owned(), json!("root"));
    let result = resolve_includes(root_config, &root).unwrap();
    assert_eq!(result.get("items"), Some(&json!(["root", "included"])));
    assert_eq!(result.get("value"), Some(&json!("included")));
}

#[test]
fn rejects_malformed_entries() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("root.yaml");
    let error = resolve_includes(
        ConfigMap::from_iter([("include".to_owned(), json!("file.yaml"))]),
        &root,
    )
    .unwrap_err();
    assert!(matches!(error, IncludeError::Entries));
}
