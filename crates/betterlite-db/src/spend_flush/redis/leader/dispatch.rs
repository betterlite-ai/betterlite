use std::error::Error as StdError;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::entity_spend_commit::EntityTableWriter;
use crate::spend_commit::daily::DailyChunkWriter;
use crate::spend_commit::policy::SpendRetryPolicy;
use crate::spend_flush::discovery::ToolDiscoveryWriter;
use crate::spend_flush::memory::MemoryFlushError;
use crate::spend_flush::memory::{DailySpendQueues, MemorySpendFlush};
use crate::spend_flush::service::ServiceFuture;
use crate::spend_flush::window::WindowSpendCommit;

use super::super::parts_with_retries;

// One buffer-mode branch of db_update_spend_transaction_handler: it commits every memory queue it owns with the retry count it was handed.
pub trait SpendFlushPass: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn commit_spend_updates(&self, retries: u32) -> ServiceFuture<'_, Result<(), Self::Error>>;
}

#[derive(Debug, thiserror::Error)]
pub enum SpendUpdateError<RedisFailure, MemoryFailure>
where
    RedisFailure: StdError + Send + Sync + 'static,
    MemoryFailure: StdError + Send + Sync + 'static,
{
    #[error("the redis spend flush failed")]
    Redis(#[source] RedisFailure),
    #[error("the in-memory spend flush failed")]
    Memory(#[source] MemoryFailure),
}

// db_update_spend_transaction_handler's branch on the loaded use_redis_transaction_buffer policy; the branch not taken reaches none of its ports.
pub enum SpendUpdateTransactionHandler<'run, Redis, Memory> {
    Redis(&'run Redis),
    Memory(&'run Memory),
}

impl<'run, Redis, Memory> SpendUpdateTransactionHandler<'run, Redis, Memory>
where
    Redis: SpendFlushPass,
    Memory: SpendFlushPass,
{
    #[must_use]
    pub const fn select(
        use_redis_transaction_buffer: bool,
        redis: &'run Redis,
        memory: &'run Memory,
    ) -> Self {
        if use_redis_transaction_buffer {
            Self::Redis(redis)
        } else {
            Self::Memory(memory)
        }
    }

    pub async fn commit_spend_updates(
        &self,
        retries: u32,
    ) -> Result<(), SpendUpdateError<Redis::Error, Memory::Error>> {
        match self {
            Self::Redis(redis) => redis
                .commit_spend_updates(retries)
                .await
                .map_err(SpendUpdateError::Redis),
            Self::Memory(memory) => memory
                .commit_spend_updates(retries)
                .await
                .map_err(SpendUpdateError::Memory),
        }
    }
}

// _commit_spend_updates_to_db_without_redis_buffer behind the dispatch: the same memory pass, with the adapter's retry count in its policy and daily parts.
impl<EntityWriter, Cache, DailyWriter, Window, Discovery> SpendFlushPass
    for MemorySpendFlush<'_, EntityWriter, Cache, DailyWriter, Window, Discovery>
where
    EntityWriter: EntityTableWriter,
    Cache: ResetCache,
    DailyWriter: DailyChunkWriter + 'static,
    Window: WindowSpendCommit,
    Discovery: ToolDiscoveryWriter,
{
    type Error = MemoryFlushError<EntityWriter::Error, Cache::Failure>;

    fn commit_spend_updates(&self, retries: u32) -> ServiceFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let parts = parts_with_retries(self.daily_parts, retries);
            let policy = SpendRetryPolicy {
                retries,
                ..self.policy
            };
            let flush = MemorySpendFlush {
                entity_queue: self.entity_queue,
                entity_writer: self.entity_writer,
                entity_cache: self.entity_cache,
                policy,
                daily: DailySpendQueues {
                    user: self.daily.user,
                    team: self.daily.team,
                    org: self.daily.org,
                    end_user: self.daily.end_user,
                    agent: self.daily.agent,
                },
                daily_commits: self.daily_commits,
                daily_parts: &parts,
                window_queue: self.window_queue,
                window: self.window,
                discovery_queue: self.discovery_queue,
                discovery: self.discovery,
                shutdown: self.shutdown,
            };
            flush.flush().await
        })
    }
}
