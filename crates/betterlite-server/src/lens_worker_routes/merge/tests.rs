#![cfg(test)]

use betterlite_lens::models::finding::{
    Evidence, FindingDraft, FindingStatus, Kind, Priority, Role,
};
use betterlite_lens::models::job::Lens;

use crate::lens_access::tests::support::{finding, now};
use crate::lens_worker_routes::merge::merge_results;
use crate::lens_worker_routes::tests::support::base_lens;

fn draft_with(
    title: &str,
    check_id: &str,
    existing: Option<&str>,
    runs: &[(&str, Role)],
) -> FindingDraft {
    FindingDraft {
        title: title.to_owned(),
        description: "The checkout retried twice".to_owned(),
        check_id: check_id.to_owned(),
        kind: Kind::Issue,
        priority: Priority::Medium,
        suggestion: String::new(),
        limitation: String::new(),
        evidence: runs
            .iter()
            .map(|(run, role)| Evidence {
                execution_id: (*run).to_owned(),
                span_id: "s1".to_owned(),
                quote: "boom".to_owned(),
                role: *role,
            })
            .collect(),
        existing_finding_id: existing.map(str::to_owned),
    }
}

fn draft_of(title: &str, check_id: &str) -> FindingDraft {
    draft_with(title, check_id, None, &[])
}

#[test]
fn no_drafts_leave_the_lens_alone() {
    let mut lens = base_lens("l1");
    lens.findings = vec![finding("f1")];
    let merged = merge_results(&lens, &[], 1, now());
    assert_eq!(merged, lens);
}

#[test]
fn the_last_draft_comes_first() {
    let lens = base_lens("l1");
    let drafts = vec![
        draft_of("First finding", "c1"),
        draft_of("Second finding", "c1"),
    ];
    let merged = merge_results(&lens, &drafts, 1, now());
    assert_eq!(merged.findings.len(), 2);
    assert_eq!(merged.findings[0].title, "Second finding");
    assert_eq!(merged.findings[1].title, "First finding");
}

#[test]
fn a_named_old_finding_is_merged_in_place() {
    let original = finding("f1");
    let mut lens = base_lens("l1");
    lens.findings = vec![original.clone()];
    let draft = draft_with(
        "Checkout retries",
        "c1",
        Some("f1"),
        &[("t0", Role::Support)],
    );
    let merged = merge_results(&lens, &[draft], 5, now());
    assert_eq!(merged.findings.len(), 1);
    assert_eq!(merged.findings[0].id, original.id);
    assert_eq!(merged.findings[0].title, original.title);
    assert_eq!(merged.findings[0].first_seen, original.first_seen);
    assert_eq!(merged.findings[0].last_seen, now());
    assert_eq!(merged.findings[0].revision, original.revision);
    assert_eq!(merged.findings[0].occurrences, ["t0"]);
}

#[test]
fn a_repeated_identity_merges_into_one_finding() {
    let lens = base_lens("l1");
    let drafts = vec![
        draft_with("Checkout retries", "c1", None, &[("t0", Role::Support)]),
        draft_with("Checkout retries", "c1", None, &[("t1", Role::Support)]),
    ];
    let merged = merge_results(&lens, &drafts, 4, now());
    assert_eq!(merged.findings.len(), 1);
    assert_eq!(merged.findings[0].revision, 4);
    assert_eq!(merged.findings[0].occurrences.len(), 2);
    assert!(
        merged.findings[0]
            .occurrences
            .iter()
            .any(|run| run.as_str() == "t0")
    );
    assert!(
        merged.findings[0]
            .occurrences
            .iter()
            .any(|run| run.as_str() == "t1")
    );
}

#[test]
fn older_findings_stay_after_the_merged_one() {
    let mut lens = base_lens("l1");
    lens.findings = vec![finding("f1")];
    let merged = merge_results(&lens, &[draft_of("Checkout retries", "c1")], 1, now());
    assert_eq!(merged.findings.len(), 2);
    assert_eq!(merged.findings[0].title, "Checkout retries");
    assert_eq!(merged.findings[1].id, "f1");
}

#[test]
fn a_resolved_finding_reopens_on_a_new_run() {
    let mut original = finding("f1");
    original.status = FindingStatus::Resolved;
    let mut lens = base_lens("l1");
    lens.findings = vec![original];
    let draft = draft_with(
        "Checkout retries",
        "c1",
        Some("f1"),
        &[("t0", Role::Support)],
    );
    let merged = merge_results(&lens, &[draft], 1, now());
    assert_eq!(merged.findings[0].status, FindingStatus::Open);
}

#[test]
fn a_resolved_finding_stays_resolved_without_new_evidence() {
    let mut original = finding("f1");
    original.status = FindingStatus::Resolved;
    original.occurrences = vec!["t0".to_owned()];
    let mut lens = base_lens("l1");
    lens.findings = vec![original.clone()];
    let draft = draft_with(
        "Checkout retries",
        "c1",
        Some("f1"),
        &[("t0", Role::Support)],
    );
    let merged = merge_results(&lens, &[draft], 1, now());
    assert_eq!(merged.findings[0].status, FindingStatus::Resolved);
    assert_eq!(merged.findings[0].last_seen, original.last_seen);
}

#[test]
fn counterexample_evidence_is_not_an_occurrence() {
    let lens = base_lens("l1");
    let draft = draft_with(
        "Checkout retries",
        "c1",
        None,
        &[("t0", Role::Counterexample)],
    );
    let merged: Lens = merge_results(&lens, &[draft], 1, now());
    assert!(merged.findings[0].occurrences.is_empty());
    assert_eq!(merged.findings[0].evidence.len(), 1);
}
