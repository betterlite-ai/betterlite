#![cfg(test)]

use std::fs::{self, File, TryLockError};
use std::os::unix::process::CommandExt as _;
use std::process::{self, Command, Stdio};
use std::thread;
use std::time::Duration;

use super::{hold, stop_orphan};

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
    stop_orphan(log_dir.path()).unwrap();
    drop(hold(log_dir.path()).unwrap());
    stop_orphan(log_dir.path()).unwrap();
    let file = File::open(log_dir.path().join("gate.lock")).unwrap();
    file.try_lock().unwrap();
}

#[test]
fn an_orphaned_holder_is_interrupted_with_its_whole_group() {
    let log_dir = tempfile::tempdir().unwrap();
    let path = log_dir.path().join("gate.lock");
    // the shell exits at once, so init reaps the holder the way it reaps a hook whose worker died
    let mut shell = Command::new("bash")
        .args(["-c", r#"flock "$0" sleep 600 >/dev/null 2>&1 &"#])
        .arg(&path)
        .process_group(0)
        .spawn()
        .unwrap();
    let group = shell.id();
    shell.wait().unwrap();
    for _ in 0_u8..50 {
        let held = File::open(&path)
            .is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)));
        if held {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    fs::write(&path, group.to_string()).unwrap();
    stop_orphan(log_dir.path()).unwrap();
    let alive = Command::new("kill")
        .args(["-s", "0", "--", &format!("-{group}")])
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!alive.success());
    File::open(&path).unwrap().try_lock().unwrap();
}
