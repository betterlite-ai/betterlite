use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use rust_decimal::Decimal;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::entity_spend_commit::EntityTableWriter;
use crate::entity_spend_commit::progress::{EntityCommitError, EntitySpendCommit};
use crate::spend_commit::daily::DailyChunkWriter;
use crate::spend_commit::policy::{SpendRetryPolicy, SpendUpdateFailure};
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions};
use crate::spend_flush::daily::{DailyCommitParts, DailyFlushError, flush_daily_spend_queue};
use crate::spend_flush::discovery::{ToolDiscoveryWriter, flush_tool_discovery_queue};
use crate::spend_flush::window::{WindowSpendCommit, flush_window_spend_queue};
use crate::spend_queues::SpendQueueError;
use crate::spend_queues::daily::DailySpendUpdateQueue;
use crate::spend_queues::entity::{SpendAmount, SpendUpdateQueue, SpendUpdateTransactions};
use crate::tool_discovery::ToolDiscoveryQueue;
use crate::window_queue::WindowQueueError;
use crate::window_queue::queue::WindowSpendUpdateQueue;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum MemoryFlushError<Write, Cache>
where
    Write: SpendUpdateFailure,
    Cache: StdError + Send + Sync + 'static,
{
    #[error("the in-memory spend update queue could not be drained")]
    EntityQueue(#[source] SpendQueueError),
    #[error("the entity spend commit failed")]
    Entity(#[source] EntityCommitError<Write, Cache>),
    #[error(transparent)]
    Daily(#[from] DailyFlushError),
    #[error("the budget window spend queue could not be drained")]
    WindowQueue(#[source] WindowQueueError),
}

// The five daily queues the memory pass commits, in the source's order. The tag queue is committed by its own longer-interval job.
pub struct DailySpendQueues<'run> {
    pub user: &'run Arc<DailySpendUpdateQueue>,
    pub team: &'run Arc<DailySpendUpdateQueue>,
    pub org: &'run Arc<DailySpendUpdateQueue>,
    pub end_user: &'run Arc<DailySpendUpdateQueue>,
    pub agent: &'run Arc<DailySpendUpdateQueue>,
}

impl<'run> DailySpendQueues<'run> {
    #[must_use]
    pub const fn in_flush_order(
        &self,
    ) -> [(DailySpendEntity, &'run Arc<DailySpendUpdateQueue>); 5] {
        [
            (DailySpendEntity::User, self.user),
            (DailySpendEntity::Team, self.team),
            (DailySpendEntity::Org, self.org),
            (DailySpendEntity::EndUser, self.end_user),
            (DailySpendEntity::Agent, self.agent),
        ]
    }
}

// _commit_spend_updates_to_db_without_redis_buffer: the entity aggregate first, then every daily queue, then the budget windows, then tool discovery.
pub struct MemorySpendFlush<'run, EntityWriter, Cache, DailyWriter, Window, Discovery> {
    pub entity_queue: &'run SpendUpdateQueue,
    pub entity_writer: &'run EntityWriter,
    pub entity_cache: &'run Cache,
    pub policy: SpendRetryPolicy<'run>,
    pub daily: DailySpendQueues<'run>,
    pub daily_commits: &'run TaskTracker,
    pub daily_parts: &'run Arc<DailyCommitParts<DailyWriter>>,
    pub window_queue: &'run WindowSpendUpdateQueue,
    pub window: &'run Window,
    pub discovery_queue: &'run ToolDiscoveryQueue,
    pub discovery: &'run Discovery,
    pub shutdown: &'run CancellationToken,
}

impl<EntityWriter, Cache, DailyWriter, Window, Discovery>
    MemorySpendFlush<'_, EntityWriter, Cache, DailyWriter, Window, Discovery>
where
    EntityWriter: EntityTableWriter,
    Cache: ResetCache,
    DailyWriter: DailyChunkWriter + 'static,
    Window: WindowSpendCommit,
    Discovery: ToolDiscoveryWriter,
{
    pub async fn flush(&self) -> Result<(), MemoryFlushError<EntityWriter::Error, Cache::Failure>> {
        self.commit_entity().await?;
        for (entity, queue) in self.daily.in_flush_order() {
            flush_daily_spend_queue(
                queue,
                entity,
                self.daily_parts,
                self.daily_commits,
                self.shutdown,
            )
            .await
            .map_err(MemoryFlushError::Daily)?;
        }
        flush_window_spend_queue(self.window_queue, self.window)
            .await
            .map_err(MemoryFlushError::WindowQueue)?;
        flush_tool_discovery_queue(self.discovery_queue, self.discovery).await;
        Ok(())
    }

    // _commit_spend_updates_to_db over the drained entity aggregate: the source never puts these increments back, so a failed table ends the pass.
    async fn commit_entity(
        &self,
    ) -> Result<(), MemoryFlushError<EntityWriter::Error, Cache::Failure>> {
        let transactions = self
            .entity_queue
            .flush_aggregated_transactions()
            .map_err(MemoryFlushError::EntityQueue)?;
        let tables = spend_table_transactions(&transactions);
        let commit = EntitySpendCommit {
            writer: self.entity_writer,
            policy: self.policy,
            cache: self.entity_cache,
        };
        commit
            .commit_spend_updates(&tables, |_table| {})
            .await
            .map_err(MemoryFlushError::Entity)
    }
}

// get_aggregated_db_spend_update_transactions: one table entry per entity kind, empty when nothing was queued for it.
#[must_use]
pub fn spend_table_transactions(transactions: &SpendUpdateTransactions) -> SpendTableTransactions {
    let mut tables = SpendTableTransactions::new();
    let buckets = [
        (SpendTable::UserList, &transactions.user_list_transactions),
        (
            SpendTable::EndUserList,
            &transactions.end_user_list_transactions,
        ),
        (SpendTable::KeyList, &transactions.key_list_transactions),
        (SpendTable::TeamList, &transactions.team_list_transactions),
        (
            SpendTable::TeamMemberList,
            &transactions.team_member_list_transactions,
        ),
        (SpendTable::OrgList, &transactions.org_list_transactions),
        (
            SpendTable::OrgMemberList,
            &transactions.org_member_list_transactions,
        ),
        (
            SpendTable::ProjectList,
            &transactions.project_list_transactions,
        ),
        (SpendTable::TagList, &transactions.tag_list_transactions),
        (
            SpendTable::ModelAccessGroupList,
            &transactions.model_access_group_spend,
        ),
        (SpendTable::AgentList, &transactions.agent_list_transactions),
    ];
    for (table, bucket) in buckets {
        tables.insert(table, Some(amounts(bucket)));
    }
    tables
}

fn amounts(bucket: &BTreeMap<String, SpendAmount>) -> BTreeMap<String, Decimal> {
    bucket
        .iter()
        .map(|(entity_id, amount)| (entity_id.clone(), amount.0))
        .collect()
}
