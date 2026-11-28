use betterlite_lens::models::finding::FindingDraft;
use betterlite_lens::models::job::Lens;
use betterlite_lens::state::finding_identity::merge_finding;
use chrono::{DateTime, Utc};

// inference.merge_results: merge each draft against the lens the drafts before it left, so the last one leads
#[must_use]
pub(crate) fn merge_results(
    lens: &Lens,
    drafts: &[FindingDraft],
    revision: i64,
    now: DateTime<Utc>,
) -> Lens {
    drafts.iter().fold(lens.clone(), |current, draft| {
        let finding = merge_finding(&current, draft, revision, now);
        let merged_id = finding.id.clone();
        let mut findings = Vec::with_capacity(current.findings.len() + 1);
        findings.push(finding);
        findings.extend(
            current
                .findings
                .iter()
                .filter(|older| older.id != merged_id)
                .cloned(),
        );
        Lens {
            findings,
            ..current
        }
    })
}

mod tests;
