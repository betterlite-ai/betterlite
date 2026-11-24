pub mod jobs;
pub mod leader;
pub mod lease;
pub mod tags;

pub(crate) mod fakes;

use std::sync::Arc;

use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::spend_commit::daily::{DailyChunkCommit, DailyChunkWriter};
use crate::spend_commit::phase::CommitPhase;
use crate::spend_commit::policy::SpendRetryPolicy;
use crate::spend_flush::daily::DailyCommitParts;
use crate::spend_queues::daily::DailySpendUpdate;

// DB_SPEND_UPDATE_JOB_NAME and DB_DAILY_TAG_SPEND_UPDATE_JOB_NAME of litellm/constants.py: the cron job ids the two Redis flushers elect on.
pub const DB_SPEND_UPDATE_JOB_NAME: &str = "db_spend_update_job";
pub const DAILY_TAG_SPEND_JOB_NAME: &str = "db_daily_tag_spend_update_job";

// The daily commit parts a job adapter's pinned retry count rebuilds, so a Redis flusher commits with the count it was handed.
pub(crate) fn parts_with_retries<Writer>(
    parts: &DailyCommitParts<Writer>,
    retries: u32,
) -> Arc<DailyCommitParts<Writer>> {
    Arc::new(DailyCommitParts {
        writer: Arc::clone(&parts.writer),
        retries,
        clock: Arc::clone(&parts.clock),
        jitter: Arc::clone(&parts.jitter),
        pause: Arc::clone(&parts.pause),
        failures: Arc::clone(&parts.failures),
    })
}

// _update_daily_spend for one drained map: the writer removes every key it settled, so the caller restores exactly what the commit left behind.
pub(crate) async fn commit_daily<Writer>(
    parts: &DailyCommitParts<Writer>,
    retries: u32,
    phase: Arc<CommitPhase>,
    entity: DailySpendEntity,
    pending: &mut DailySpendUpdate,
) -> Result<(), Writer::Error>
where
    Writer: DailyChunkWriter,
{
    let policy = SpendRetryPolicy {
        retries,
        clock: &*parts.clock,
        jitter: &*parts.jitter,
        pause: &*parts.pause,
        failures: &*parts.failures,
    };
    let commit = DailyChunkCommit {
        writer: &*parts.writer,
        policy,
        phase,
    };
    commit.update_daily_spend(entity, pending).await
}
