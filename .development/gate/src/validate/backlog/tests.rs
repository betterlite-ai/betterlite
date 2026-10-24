#![cfg(test)]

use crate::spec::backlog::{BacklogEntry, Resolution, Status};
use crate::validate::backlog::ticket_problems;

fn entry() -> BacklogEntry {
    BacklogEntry {
        id: "B004".to_owned(),
        milestone: "M1".to_owned(),
        title: "Probe".to_owned(),
        status: Status::Todo,
        depends_on: Vec::new(),
        file: None,
        covers: Vec::new(),
        parent: None,
        split_from: None,
        resolution: None,
        superseded_by: None,
        blocked_reason: None,
        pr: None,
    }
}

fn reports(probe: &BacklogEntry, expected: &str) -> bool {
    ticket_problems(probe, &["B004"])
        .iter()
        .any(|problem| problem.contains(expected))
}

#[test]
fn a_done_entry_needs_a_resolution() {
    let mut probe = entry();
    probe.status = Status::Done;
    assert!(reports(&probe, "a done entry has a resolution"));
}

#[test]
fn a_merged_entry_needs_its_pr() {
    let mut probe = entry();
    probe.status = Status::Done;
    probe.resolution = Some(Resolution::Merged);
    assert!(reports(&probe, "a merged entry names its pr"));
}

#[test]
fn a_duplicate_names_what_replaces_it() {
    let mut probe = entry();
    probe.status = Status::Done;
    probe.resolution = Some(Resolution::Duplicate);
    assert!(reports(&probe, "superseded_by is set exactly"));
}

#[test]
fn a_blocked_entry_says_why() {
    let mut probe = entry();
    probe.status = Status::Blocked;
    assert!(reports(&probe, "blocked_reason is set exactly"));
}

#[test]
fn a_parent_must_be_in_the_backlog() {
    let mut probe = entry();
    probe.parent = Some("B999".to_owned());
    assert!(reports(
        &probe,
        "parent B999 is itself or not in the backlog"
    ));
}

#[test]
fn a_merged_entry_with_its_pr_passes() {
    let mut probe = entry();
    probe.status = Status::Done;
    probe.resolution = Some(Resolution::Merged);
    probe.pr = Some(43);
    assert!(ticket_problems(&probe, &["B004"]).is_empty());
}
