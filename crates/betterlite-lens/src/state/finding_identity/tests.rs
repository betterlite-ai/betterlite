#![cfg(test)]

use chrono::{DateTime, Utc};
use rstest::rstest;

use crate::models::finding::{FindingDraft, FindingStatus, Kind};
use crate::models::job::Lens;
use crate::state::finding_identity::merge_finding;

fn lens() -> Lens {
    serde_json::from_str(include_str!("../fixtures/lens.json")).unwrap()
}

fn draft() -> FindingDraft {
    serde_json::from_str(include_str!("../fixtures/draft.json")).unwrap()
}

fn at(text: &str) -> DateTime<Utc> {
    text.parse().unwrap()
}

#[rstest]
#[case::issue(Kind::Issue, "1cc65b3b75d75cc977f1a629")]
#[case::pattern(Kind::Pattern, "36e25fb06dad166cf914b8a5")]
fn a_new_finding_hashes_lens_check_kind_and_title(#[case] kind: Kind, #[case] id: &str) {
    let mut fresh = lens();
    fresh.findings.clear();
    let mut found = draft();
    found.kind = kind;
    let finding = merge_finding(&fresh, &found, 7, at("2026-01-15T10:00:00Z"));
    assert_eq!(finding.id, id);
    assert_eq!(
        (finding.status, finding.revision, finding.reason.as_str()),
        (FindingStatus::Open, 7, "")
    );
    assert_eq!(
        (finding.first_seen, finding.last_seen),
        (at("2026-01-15T10:00:00Z"), at("2026-01-15T10:00:00Z"))
    );
    assert_eq!(finding.evidence, found.evidence);
}

#[test]
fn new_support_reopens_a_resolved_finding_under_its_legacy_id() {
    let now = at("2026-01-15T10:00:00Z");
    let finding = merge_finding(&lens(), &draft(), 7, now);
    assert_eq!(finding.id, "eff9663127b1652e37394894");
    assert_eq!(
        (finding.status, finding.revision, finding.last_seen),
        (FindingStatus::Open, 2, now)
    );
    assert_eq!(finding.occurrences, ["exec-1", "exec-2"]);
    assert_eq!(
        (finding.title.as_str(), finding.first_seen),
        ("Slow replies", at("2026-01-11T08:00:00Z"))
    );
}

#[test]
fn known_support_leaves_a_resolved_finding_resolved() {
    let mut known = draft();
    known.evidence[0].execution_id = "exec-1".to_owned();
    let finding = merge_finding(&lens(), &known, 7, at("2026-01-15T10:00:00Z"));
    assert_eq!(
        (finding.status, finding.last_seen),
        (FindingStatus::Resolved, at("2026-01-12T08:00:00Z"))
    );
    assert_eq!(finding.occurrences, ["exec-1"]);
}

#[test]
fn occurrences_count_support_but_never_counterexamples() {
    let mut fresh = lens();
    fresh.findings.clear();
    let finding = merge_finding(&fresh, &draft(), 7, at("2026-01-15T10:00:00Z"));
    assert_eq!(finding.occurrences, ["exec-2"]);
}

#[rstest]
#[case::named_by_the_worker(Some("eff9663127b1652e37394894"), "Unrelated title", Kind::Issue, true)]
#[case::other_kind(None, "Slow Replies", Kind::Pattern, false)]
#[case::other_title(None, "Fast replies", Kind::Issue, false)]
fn a_finding_matches_by_id_kind_and_check(
    #[case] existing: Option<&str>,
    #[case] title: &str,
    #[case] kind: Kind,
    #[case] merged: bool,
) {
    let mut found = draft();
    found.existing_finding_id = existing.map(str::to_owned);
    found.title = title.to_owned();
    found.kind = kind;
    let finding = merge_finding(&lens(), &found, 7, at("2026-01-15T10:00:00Z"));
    assert_eq!(finding.id == "eff9663127b1652e37394894", merged);
}
