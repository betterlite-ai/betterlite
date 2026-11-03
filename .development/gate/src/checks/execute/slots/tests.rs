#![cfg(test)]

use std::fs;
use std::io;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[test]
fn a_busy_build_slot_queues_the_next_toy_compiler_and_releases_after_failure() {
    let folder = tempfile::tempdir().unwrap();
    let held = super::acquire(folder.path(), 1, &mut io::sink()).unwrap();
    let root = folder.path().to_owned();
    let build = thread::spawn(move || {
        let _lease = super::acquire(&root, 1, &mut io::sink()).unwrap();
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
    let _next = super::acquire(folder.path(), 1, &mut io::sink()).unwrap();
}

#[test]
fn a_gate_waiting_for_a_build_slot_says_so_once() {
    let folder = tempfile::tempdir().unwrap();
    let mut quiet = Vec::new();
    let held = super::acquire(folder.path(), 1, &mut quiet).unwrap();
    assert!(quiet.is_empty());
    let release = thread::spawn(move || {
        thread::sleep(Duration::from_millis(350));
        drop(held);
    });
    let mut notice = Vec::new();
    let _lease = super::acquire(folder.path(), 1, &mut notice).unwrap();
    release.join().unwrap();
    assert_eq!(
        String::from_utf8(notice).unwrap(),
        format!(
            "gate: waiting for a build slot, all 1 in {} are busy\n",
            folder.path().display()
        )
    );
}
