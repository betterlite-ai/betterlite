use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use crate::models::finding::Role;
use crate::models::finding::{Finding, FindingDraft};
use crate::models::job::Lens;
use crate::state::finding_identity::merge_finding;

mod tests;

#[must_use]
pub fn snapshot_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: i64,
    now: DateTime<Utc>,
) -> Finding {
    let merged = merge_finding(lens, draft, revision, now);
    Finding {
        title: draft.title.clone(),
        description: draft.description.clone(),
        check_id: draft.check_id.clone(),
        kind: draft.kind,
        priority: draft.priority,
        suggestion: draft.suggestion.clone(),
        limitation: draft.limitation.clone(),
        evidence: draft.evidence.clone(),
        existing_finding_id: draft.existing_finding_id.clone(),
        revision,
        first_seen: now,
        last_seen: now,
        occurrences: BTreeSet::from_iter(draft.evidence.iter().filter_map(|evidence| {
            (evidence.role == Role::Support).then_some(evidence.execution_id.clone())
        }))
        .into_iter()
        .collect(),
        status: merged.status,
        reason: merged.reason,
        ..merged
    }
}
