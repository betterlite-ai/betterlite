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

fn identity_parts(lens: &Lens, draft: &FindingDraft) -> (String, String) {
    let kind = match draft.kind {
        Kind::Issue => "issue",
        Kind::Pattern => "pattern",
    };
    let title = draft.title.to_lowercase();
    let identity = short_digest(&format!("{}:{}:{kind}:{title}", lens.id, draft.check_id));
    // findings stored before the kind joined the hash keep their old id
    let legacy = short_digest(&format!("{}:{}:{title}", lens.id, draft.check_id));
    (identity, legacy)
}

fn matching_finding<'finding>(
    lens: &'finding Lens,
    draft: &FindingDraft,
    identity: &str,
    legacy: &str,
) -> Option<&'finding Finding> {
    for finding in &lens.findings {
        let named = draft.existing_finding_id.as_ref() == Some(&finding.id);
        if (named || finding.id == identity || finding.id == legacy)
            && finding.kind == draft.kind
            && finding.check_id == draft.check_id
        {
            return Some(finding);
        }
    }
    None
}

#[must_use]
pub fn merge_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: i64,
    now: DateTime<Utc>,
) -> Finding {
    let (identity, legacy) = identity_parts(lens, draft);
    let supporting = BTreeSet::from_iter(draft.evidence.iter().filter_map(|evidence| {
        (evidence.role == Role::Support).then_some(evidence.execution_id.as_str())
    }));
    let Some(previous) = matching_finding(lens, draft, &identity, &legacy) else {
        return Finding {
            title: draft.title.clone(),
            description: draft.description.clone(),
            check_id: draft.check_id.clone(),
            kind: draft.kind,
            priority: draft.priority,
            suggestion: draft.suggestion.clone(),
            limitation: draft.limitation.clone(),
            evidence: draft.evidence.clone(),
            existing_finding_id: draft.existing_finding_id.clone(),
            id: identity,
            status: FindingStatus::Open,
            reason: String::new(),
            first_seen: now,
            last_seen: now,
            occurrences: supporting.into_iter().map(str::to_owned).collect(),
            revision,
        };
    };
    let seen: BTreeSet<&str> = previous.occurrences.iter().map(String::as_str).collect();
    let recurred = !supporting.is_subset(&seen);
    Finding {
        last_seen: if recurred { now } else { previous.last_seen },
        occurrences: seen.union(&supporting).map(|id| (*id).to_owned()).collect(),
        evidence: merge_evidence(&previous.evidence, &draft.evidence),
        status: if previous.status == FindingStatus::Resolved && recurred {
            FindingStatus::Open
        } else {
            previous.status
        },
        ..previous.clone()
    }
}
