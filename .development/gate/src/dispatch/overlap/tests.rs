#![cfg(test)]

use std::fs;
use std::path::Path;

use super::{overlaps, probes};
use crate::development::Development;
use crate::git::git;
use crate::spec::backlog::{Backlog, BacklogEntry, Status};
use crate::spec::task::Task;
use crate::yaml::read_yaml;

#[test]
fn a_literal_path_is_its_own_probe() {
    let paths = ["crates/betterlite-config/src/raw.rs".to_owned()];
    assert_eq!(probes(&paths), ["crates/betterlite-config/src/raw.rs"]);
}

#[test]
fn a_glob_probes_inside_its_folder() {
    let paths = ["crates/betterlite-http/src/proxy/**".to_owned()];
    assert_eq!(probes(&paths), ["crates/betterlite-http/src/proxy/probe"]);
}

#[test]
fn braces_expand_before_the_glob_cut() {
    let paths = ["crates/betterlite-http/src/{lib,proxy}.rs".to_owned()];
    assert_eq!(
        probes(&paths),
        [
            "crates/betterlite-http/src/lib.rs",
            "crates/betterlite-http/src/proxy.rs"
        ]
    );
}

#[test]
fn a_path_two_running_tasks_share_stops_clashing_once_overlap_allowed_lists_it() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let file = source.join("tasks/B000-workspace.yaml");
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    git(root, &["init", "--quiet", "-b", "main"]).unwrap();
    fs::create_dir_all(root.join(".development/tasks")).unwrap();
    fs::copy(&file, root.join(".development/tasks/running.yaml")).unwrap();
    let mut running: BacklogEntry = read_yaml(&source.join("backlog/B000.yaml")).unwrap();
    running.status = Status::InProgress;
    running.file = Some("tasks/running.yaml".to_owned());
    let mut development = Development {
        root: root.to_owned(),
        catalog: read_yaml(&source.join("checks.yaml")).unwrap(),
        backlog: Backlog {
            tasks: vec![running],
        },
    };
    let mut task: Task = read_yaml(&file).unwrap();
    task.id = "B001".to_owned();
    task.scope.paths = vec!["README.md".to_owned()];
    development.catalog.settings.overlap_allowed.clear();
    assert_eq!(overlaps(&development, &task).unwrap().len(), 1);
    development.catalog.settings.overlap_allowed = vec!["README.md".to_owned()];
    assert!(overlaps(&development, &task).unwrap().is_empty());
}
