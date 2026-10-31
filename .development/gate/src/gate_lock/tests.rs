#![cfg(test)]

use std::fs::{self, File, TryLockError};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::Path;
use std::process::{self, Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use super::{GRACE, hold, stop_orphan};

fn hold_in_own_group(path: &Path, script: &str) -> Child {
    let child = Command::new("bash")
        .args(["-c", script])
        .arg(path)
        .process_group(0)
        .spawn()
        .unwrap();
    for _ in 0_u8..50 {
        let held = File::open(path)
            .is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
        if held {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    fs::write(path, child.id().to_string()).unwrap();
    child
}

#[test]
fn a_held_gate_lock_names_the_holders_process_group() {
    let log_dir = tempfile::tempdir().unwrap();
    let held = hold(log_dir.path()).unwrap();
    let path = log_dir.path().join("gate.lock");
    let ps = Command::new("ps")
        .args(["-o", "pgid=", "-p", &process::id().to_string()])
        .output()
        .unwrap();
    let group = String::from_utf8(ps.stdout).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), group.trim());
    let other = Command::new("flock")
        .arg("-n")
        .arg(&path)
        .arg("true")
        .status()
        .unwrap();
    assert!(!other.success());
    drop(held);
}

#[test]
fn a_missing_or_free_gate_lock_is_left_alone() {
    let log_dir = tempfile::tempdir().unwrap();
    stop_orphan(log_dir.path(), GRACE).unwrap();
    let mut named = Command::new("sleep")
        .arg("600")
        .process_group(0)
        .spawn()
        .unwrap();
    fs::write(log_dir.path().join("gate.lock"), named.id().to_string()).unwrap();
    stop_orphan(log_dir.path(), GRACE).unwrap();
    named.kill().unwrap();
    assert_eq!(named.wait().unwrap().signal(), Some(9_i32));
}

#[test]
fn an_orphaned_holder_is_interrupted_with_its_whole_group() {
    let log_dir = tempfile::tempdir().unwrap();
    let path = log_dir.path().join("gate.lock");
    let mut holder = hold_in_own_group(&path, r#"exec flock "$0" sleep 600"#);
    let started = Instant::now();
    stop_orphan(log_dir.path(), GRACE).unwrap();
    // flock's sleep holds the lock too, so an interrupt that missed it would run into the grace
    assert!(started.elapsed() < GRACE);
    assert_eq!(holder.wait().unwrap().signal(), Some(2_i32));
}

#[test]
fn a_holder_that_ignores_the_interrupt_is_killed_after_the_grace() {
    let log_dir = tempfile::tempdir().unwrap();
    let path = log_dir.path().join("gate.lock");
    // a job that a non-interactive shell starts with & ignores SIGINT the same way
    let mut holder = hold_in_own_group(&path, r#"trap "" INT; exec flock "$0" sleep 600"#);
    stop_orphan(log_dir.path(), Duration::from_secs(1)).unwrap();
    assert_eq!(holder.wait().unwrap().signal(), Some(9_i32));
}
