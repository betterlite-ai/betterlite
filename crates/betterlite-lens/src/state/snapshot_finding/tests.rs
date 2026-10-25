#![cfg(test)]

use chrono::{DateTime, Utc};

use crate::models::finding::{FindingDraft, FindingStatus, Priority};
use crate::models::job::Lens;
use crate::state::snapshot_finding::snapshot_finding;

#[test]
fn a_snapshot_takes_the_draft_and_restarts_its_history() {
    let lens: Lens = serde_json::from_str(include_str!("../fixtures/lens.json")).unwrap();
    let draft: FindingDraft = serde_json::from_str(include_str!("../fixtures/draft.json")).unwrap();
    let now: DateTime<Utc> = "2026-01-15T10:00:00Z".parse().unwrap();
    let finding = snapshot_finding(&lens, &draft, 9, now);
    assert_eq!(finding.id, "eff9663127b1652e37394894");
    assert_eq!(
        (finding.title.as_str(), finding.priority),
        ("Slow Replies", Priority::High)
    );
    assert_eq!(finding.description, draft.description);
    assert_eq!(finding.suggestion, draft.suggestion);
    assert_eq!(finding.evidence, draft.evidence);
    assert_eq!(
        (finding.first_seen, finding.last_seen, finding.revision),
        (now, now, 9)
    );
    assert_eq!(finding.occurrences, ["exec-2"]);
    assert_eq!(
        (finding.status, finding.reason.as_str()),
        (FindingStatus::Open, "fixed in the January release")
    );
}
