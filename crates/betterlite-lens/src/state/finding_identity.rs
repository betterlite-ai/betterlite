use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use crate::models::finding::{Finding, FindingDraft, FindingStatus, Kind, Role};
use crate::models::job::Lens;
use crate::state::evidence_cap::merge_evidence;

mod tests;

fn short_digest(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    hex::encode(digest.iter().take(12).copied().collect::<Vec<u8>>())
}

fn support(draft: &FindingDraft) -> BTreeSet<&str> {
    let supporting = draft
        .evidence
        .iter()
        .filter(|evidence| evidence.role == Role::Support);
    supporting
        .map(|evidence| evidence.execution_id.as_str())
        .collect()
}

pub(crate) fn fresh_finding(
    draft: &FindingDraft,
    id: String,
    revision: i64,
    now: DateTime<Utc>,
) -> Finding {
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
        id,
        status: FindingStatus::Open,
        reason: String::new(),
        first_seen: now,
        last_seen: now,
        occurrences: support(draft).into_iter().map(str::to_owned).collect(),
        revision,
    }
}

#[must_use]
pub fn merge_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: i64,
    now: DateTime<Utc>,
) -> Finding {
    let kind = match draft.kind {
        Kind::Issue => "issue",
        Kind::Pattern => "pattern",
    };
    let title = draft.title.to_lowercase();
    let identity = short_digest(&format!("{}:{}:{kind}:{title}", lens.id, draft.check_id));
    // findings stored before the kind joined the hash keep their old id
    let legacy = short_digest(&format!("{}:{}:{title}", lens.id, draft.check_id));
    let matched = lens.findings.iter().find(|finding| {
        let named = draft.existing_finding_id.as_ref() == Some(&finding.id);
        (named || finding.id == identity || finding.id == legacy)
            && finding.kind == draft.kind
            && finding.check_id == draft.check_id
    });
    let Some(previous) = matched else {
        return fresh_finding(draft, identity, revision, now);
    };
    let seen: BTreeSet<&str> = previous.occurrences.iter().map(String::as_str).collect();
    let supported = support(draft);
    let recurred = !supported.is_subset(&seen);
    let reopened = previous.status == FindingStatus::Resolved && recurred;
    Finding {
        last_seen: if recurred { now } else { previous.last_seen },
        occurrences: seen.union(&supported).map(|id| (*id).to_owned()).collect(),
        evidence: merge_evidence(&previous.evidence, &draft.evidence),
        status: if reopened {
            FindingStatus::Open
        } else {
            previous.status
        },
        ..previous.clone()
    }
}
