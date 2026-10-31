#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::backlog::read_backlog;
use crate::deviations::file_problems;

#[test]
fn a_log_with_what_why_and_an_https_link_passes() {
    let (backlog, _) = read_backlog(&Path::new(env!("CARGO_MANIFEST_DIR")).join("..")).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let log = folder.path().join("B003.yaml");
    fs::write(
        &log,
        "task: B003\ndeviations:\n  - what: keeps the header\n    why: LiteLLM drops it\n    link: https://example.com/a\n  - what: rejects a string ttl\n    why: the field is an int\n    link: \"\"\n",
    )
    .unwrap();
    assert!(file_problems(&log, &backlog).is_empty());
}

#[test]
fn a_log_names_its_own_task_and_every_entry_is_complete() {
    let (backlog, _) = read_backlog(&Path::new(env!("CARGO_MANIFEST_DIR")).join("..")).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let log = folder.path().join("B003.yaml");
    fs::write(
        &log,
        "task: B004\ndeviations:\n  - what: keeps the header\n    why: \" \"\n    link: http://example.com/a\n",
    )
    .unwrap();
    let shown = log.display();
    assert_eq!(
        file_problems(&log, &backlog),
        vec![
            format!("{shown} names task B004"),
            format!("{shown}: every deviation needs what, why, and an https link or an empty one"),
        ]
    );
}
