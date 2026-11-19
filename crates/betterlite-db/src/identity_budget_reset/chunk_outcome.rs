use chrono::{DateTime, Utc};

use crate::budget_permission_rows::Timestamp;
use crate::budget_reset_policy::chunk_outcome::ChunkOutcome;
use crate::budget_reset_policy::utc::count_advanced;

// _count_advanced over updated rows only, against a cutoff read after the write, so the phase can tell progress from a re-read.
#[must_use]
pub(crate) fn progress<ResetAts>(
    fetched: usize,
    reset_ats: ResetAts,
    cutoff: DateTime<Utc>,
) -> ChunkOutcome
where
    ResetAts: IntoIterator<Item = Option<DateTime<Utc>>>,
{
    let advanced = count_advanced(
        reset_ats
            .into_iter()
            .map(|reset_at| reset_at.map(|moment| Timestamp::Naive(moment.naive_utc()))),
        cutoff,
    );
    ChunkOutcome { fetched, advanced }
}

mod tests;
