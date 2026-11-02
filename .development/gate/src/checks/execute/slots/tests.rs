#![cfg(test)]

use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[test]
fn a_busy_build_slot_queues_the_next_toy_compiler_and_releases_after_failure() {
    let folder = tempfile::tempdir().unwrap();
    let held = super::acquire(folder.path(), 1).unwrap();
    let root = folder.path().to_owned();
    let build = thread::spawn(move || {
        let _lease = super::acquire(&root, 1).unwrap();
        Command::new("sh")
            .args(["-c", "printf goose > artifact; exit 7"])
            .current_dir(root)
            .status()
            .unwrap()
    });
    thread::sleep(Duration::from_millis(150));
    assert!(!folder.path().join("artifact").exists());
    drop(held);
    assert_eq!(build.join().unwrap().code(), Some(7_i32));
    assert_eq!(
        fs::read_to_string(folder.path().join("artifact")).unwrap(),
        "goose"
    );
    let _next = super::acquire(folder.path(), 1).unwrap();
}
