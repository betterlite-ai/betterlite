#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::spec::task::Task;
use crate::yaml::read_yaml;

#[test]
fn a_toy_goose_module_needs_a_writable_or_existing_integration_path() {
    let folder = tempfile::tempdir().unwrap();
    fs::create_dir_all(folder.path().join("crates/goose/src")).unwrap();
    fs::write(
        folder.path().join("crates/goose/src/lib.rs"),
        "pub fn honk() {}\n",
    )
    .unwrap();
    let mut task: Task =
        read_yaml(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../tasks/B000-workspace.yaml"))
            .unwrap();
    task.scope.paths = vec!["crates/goose/src/waddle.rs".to_owned()];
    assert!(super::check(folder.path(), &task).is_err());
    task.scope.paths.push("crates/goose/src/lib.rs".to_owned());
    super::check(folder.path(), &task).unwrap();
    task.scope.paths.pop();
    fs::write(
        folder.path().join("crates/goose/src/lib.rs"),
        "pub mod waddle;\n",
    )
    .unwrap();
    super::check(folder.path(), &task).unwrap();
}
