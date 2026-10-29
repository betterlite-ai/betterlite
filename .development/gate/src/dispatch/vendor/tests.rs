#![cfg(test)]

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::dispatch::vendor::{limit_line, limited, mark, take_seat};
use crate::spec::agents::{Agents, Vendor};
use crate::yaml::read_yaml;

fn agents() -> Agents {
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    read_yaml(&development.join("agents.yaml")).unwrap()
}

fn codex() -> Vendor {
    Vendor {
        id: "openai".to_owned(),
        limit_markers: vec!["You've hit your usage limit".to_owned()],
        max_parallel: Some(2),
    }
}

#[test]
fn a_marker_at_the_end_of_the_log_is_a_limit() {
    let folder = tempfile::tempdir().unwrap();
    let stdout = folder.path().join("attempt-1-worker.stdout");
    let log = folder.path().join("attempt-1-worker.log");
    fs::write(&stdout, "").unwrap();
    fs::write(
        &log,
        "warning: hooks enabled\nERROR: You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Nov 2nd, 2026 7:07 AM.\n",
    )
    .unwrap();
    let line = limit_line(&codex(), &[&stdout, &log]).unwrap().unwrap();
    assert!(line.starts_with("ERROR: You've hit your usage limit."));
}

#[test]
fn a_marker_the_agent_read_earlier_in_its_run_is_no_limit() {
    let folder = tempfile::tempdir().unwrap();
    let log = folder.path().join("attempt-1-worker.log");
    let later = "exec cargo nextest run\n".repeat(25);
    fs::write(
        &log,
        format!("limit_markers: [\"You've hit your usage limit\"]\n{later}"),
    )
    .unwrap();
    assert_eq!(limit_line(&codex(), &[&log]).unwrap(), None);
}

#[test]
fn a_fresh_mark_skips_the_vendor_and_an_old_one_does_not() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    assert!(!limited(git.path(), &agents, "openai").unwrap());
    mark(git.path(), "openai", "ERROR: You've hit your usage limit.").unwrap();
    assert!(limited(git.path(), &agents, "openai").unwrap());
    assert!(!limited(git.path(), &agents, "anthropic").unwrap());
    let minutes = u64::from(agents.policy.limit_retry_minutes);
    let old = SystemTime::now() - Duration::from_secs(minutes * 60 + 60);
    let path = git.path().join("betterlite/limits/openai");
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(!limited(git.path(), &agents, "openai").unwrap());
}

#[test]
fn each_run_takes_its_own_seat_up_to_the_vendor_cap() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    let first = take_seat(git.path(), &agents, &codex()).unwrap();
    let second = take_seat(git.path(), &agents, &codex()).unwrap();
    let seats = git.path().join("betterlite/seats");
    for number in 1_u32..=2_u32 {
        let held = File::open(seats.join(format!("openai-{number}"))).unwrap();
        assert!(held.try_lock().is_err(), "seat {number} is free");
    }
    drop(first);
    let again = take_seat(git.path(), &agents, &codex()).unwrap();
    drop((second, again));
    assert_eq!(fs::read_dir(&seats).unwrap().count(), 2);
}
