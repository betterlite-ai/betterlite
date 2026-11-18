use chrono::{DateTime, Utc};

use crate::budget_permission_rows::Timestamp;

// _count_advanced: only a reset_at strictly past the cutoff is progress; an aware timestamp is compared at its own instant and a naive one is read as UTC.
#[must_use]
pub fn count_advanced<Rows>(reset_ats: Rows, cutoff: DateTime<Utc>) -> usize
where
    Rows: IntoIterator<Item = Option<Timestamp>>,
{
    let mut advanced = 0_usize;
    for entry in reset_ats {
        if let Some(reset_at) = entry
            && reset_at.utc() > cutoff
        {
            advanced = advanced.saturating_add(1);
        }
    }
    advanced
}

mod tests;
