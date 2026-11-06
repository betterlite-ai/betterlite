#![cfg(test)]

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::dispatch::seat::Seat;
use crate::dispatch::vendor::{
    causes, find_vendor, first_in_line, join_line, limit_line, limited, mark, seat_free, take_seat,
};
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
        max_writers: None,
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
fn a_claude_run_logged_out_or_offline_marks_its_vendor() {
    let agents = agents();
    let claude = find_vendor(&agents, "anthropic").unwrap();
    let folder = tempfile::tempdir().unwrap();
    let stdout = folder.path().join("attempt-1-worker.stdout");
    for result in [
        "Not logged in \u{b7} Please run /login",
        "API Error: Can't reach the API server (EAI_AGAIN)",
    ] {
        let line = format!("{{\"is_error\":true,\"result\":\"{result}\",\"type\":\"result\"}}\n");
        fs::write(&stdout, line).unwrap();
        assert!(
            limit_line(claude, &[&stdout]).unwrap().is_some(),
            "{result}"
        );
    }
}

#[test]
fn a_codex_json_limit_with_a_curly_apostrophe_marks_its_vendor() {
    let agents = agents();
    let codex = find_vendor(&agents, "openai").unwrap();
    let folder = tempfile::tempdir().unwrap();
    let stdout = folder.path().join("attempt-1-worker.stdout");
    let message = "You\u{2019}ve hit your usage limit. Visit https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 10th, 2026 11:31 PM.";
    fs::write(
        &stdout,
        format!(
            "{{\"type\":\"turn.started\"}}\n{{\"type\":\"error\",\"message\":\"{message}\"}}\n{{\"type\":\"turn.failed\",\"error\":{{\"message\":\"{message}\"}}}}\n"
        ),
    )
    .unwrap();
    assert!(limit_line(codex, &[&stdout]).unwrap().is_some());
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
fn an_unreachable_or_logged_out_vendor_is_not_reported_as_a_usage_limit() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    mark(git.path(), "openai", "ERROR: You've hit your usage limit.").unwrap();
    mark(
        git.path(),
        "anthropic",
        r#"{"is_error":true,"result":"API Error: Can't reach the API server (EAI_AGAIN)","type":"result"}"#,
    )
    .unwrap();
    assert_eq!(
        causes(git.path(), &agents).unwrap(),
        "openai is at its usage limit: ERROR: You've hit your usage limit.; anthropic can't reach its API: API Error: Can't reach the API server (EAI_AGAIN)"
    );
    mark(
        git.path(),
        "anthropic",
        "Not logged in \u{b7} Please run /login",
    )
    .unwrap();
    assert!(
        causes(git.path(), &agents)
            .unwrap()
            .ends_with("; anthropic is logged out: Not logged in \u{b7} Please run /login")
    );
}

#[test]
fn each_run_takes_its_own_seat_up_to_the_vendor_cap() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    let first = take_seat(git.path(), &agents, &codex(), Seat::Worker).unwrap();
    let second = take_seat(git.path(), &agents, &codex(), Seat::Worker).unwrap();
    let seats = git.path().join("betterlite/seats");
    for number in 1_u32..=2_u32 {
        let held = File::open(seats.join(format!("openai-{number}"))).unwrap();
        assert!(held.try_lock().is_err(), "seat {number} is free");
    }
    drop(first);
    let again = take_seat(git.path(), &agents, &codex(), Seat::Worker).unwrap();
    drop((second, again));
    assert_eq!(fs::read_dir(&seats).unwrap().count(), 2);
}

#[test]
fn a_worker_also_needs_a_writer_seat_and_a_reviewer_does_not() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    let mut claude = codex();
    claude.id = "anthropic".to_owned();
    claude.max_writers = Some(1);
    let worker = take_seat(git.path(), &agents, &claude, Seat::Worker).unwrap();
    assert!(!seat_free(git.path(), &agents, &claude, Seat::Worker).unwrap());
    assert!(seat_free(git.path(), &agents, &claude, Seat::Reviewer).unwrap());
    drop(worker);
    let first = take_seat(git.path(), &agents, &claude, Seat::Reviewer).unwrap();
    let second = take_seat(git.path(), &agents, &claude, Seat::Reviewer).unwrap();
    assert!(!seat_free(git.path(), &agents, &claude, Seat::Worker).unwrap());
    drop((first, second));
    assert!(seat_free(git.path(), &agents, &claude, Seat::Worker).unwrap());
}

#[test]
fn an_expired_limit_allows_one_probe_instead_of_a_stampede() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    mark(git.path(), "openai", "You've hit your usage limit").unwrap();
    let path = git.path().join("betterlite/limits/openai");
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_hours(1))
        .unwrap();
    let probe = take_seat(git.path(), &agents, &codex(), Seat::Worker).unwrap();
    assert!(probe.is_some());
    assert!(!seat_free(git.path(), &agents, &codex(), Seat::Worker).unwrap());
    drop(probe);
    assert!(seat_free(git.path(), &agents, &codex(), Seat::Worker).unwrap());
}

#[test]
fn only_the_first_waiter_in_line_tries_for_a_seat() {
    let git = tempfile::tempdir().unwrap();
    let queue = git.path().join("seat-queue");
    let line = queue.join("anthropic-worker");
    let (first, _first_waiting) = join_line(&queue, &line).unwrap();
    let (second, _second_waiting) = join_line(&queue, &line).unwrap();
    // nobody holds this older ticket, as when its waiter stopped
    let stopped = line.join("00000000000000000001-0000000001");
    File::create(&stopped).unwrap();
    assert!(first_in_line(&line, &first).unwrap());
    assert!(!stopped.exists());
    assert!(!first_in_line(&line, &second).unwrap());
    assert_eq!(fs::read_dir(&queue).unwrap().count(), 1);
}

#[test]
fn a_seat_taken_leaves_no_ticket_behind() {
    let agents = agents();
    let git = tempfile::tempdir().unwrap();
    let held = take_seat(git.path(), &agents, &codex(), Seat::Worker).unwrap();
    assert!(held.is_some());
    let line = git.path().join("betterlite/seat-queue/openai-worker");
    assert_eq!(fs::read_dir(&line).unwrap().count(), 0);
}
