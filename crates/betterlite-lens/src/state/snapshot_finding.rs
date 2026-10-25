use chrono::{DateTime, Utc};

use crate::models::finding::{Finding, FindingDraft};
use crate::models::job::Lens;
use crate::state::finding_identity::{fresh_finding, merge_finding};

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
        status: merged.status,
        reason: merged.reason,
        ..fresh_finding(draft, merged.id, revision, now)
    }
}
