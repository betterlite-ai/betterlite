use std::future::Future;

use super::cache_invalidation::{ResetCache, invalidate_caches};

// Commits first and invalidates only after the commit succeeds: clearing a counter before the DB write lands would let a budget check read zero here while the row still holds pre-reset spend.
pub async fn commit_then_invalidate<Commit, CommitFuture, Failure, Cache>(
    cache: &Cache,
    commit: Commit,
    counter_keys: &[String],
    cache_keys: &[String],
) -> Result<(), Failure>
where
    Commit: FnOnce() -> CommitFuture + Send,
    CommitFuture: Future<Output = Result<(), Failure>> + Send,
    Failure: Send,
    Cache: ResetCache,
{
    commit().await?;
    invalidate_caches(cache, counter_keys, cache_keys).await;
    Ok(())
}

mod tests;
