use std::sync::Arc;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::entity_spend_commit::EntityTableWriter;
use crate::redis_spend::buffer::{RedisSpendError, RedisUpdateBuffer};
use crate::redis_spend::publication::{
    RedisSpendPublisher, SpendPublishError, SpendUpdateQueues, store_spend_updates_in_redis,
};
use crate::redis_spend::republication::{
    RepublicationError, UncommittedCohorts, restore_transactions_to_redis,
};
use crate::spend_commit::daily::DailyChunkWriter;
use crate::spend_commit::policy::SpendRetryPolicy;
use crate::spend_flush::daily::DailyCommitParts;
use crate::spend_flush::service::ServiceFuture;
use crate::spend_flush::window::WindowSpendCommit;
use crate::spend_queues::entity::SpendUpdateTransactions;

use self::commit::{CohortCommitError, commit_daily_slot, commit_entity, commit_window};
use self::dispatch::SpendFlushPass;
use super::DB_SPEND_UPDATE_JOB_NAME;
use super::lease::SpendPodLease;

mod commit;
pub mod dispatch;

mod tests;

// The five daily cohorts of the source's commit order; the daily tag queue has its own flusher.
const DAILY_ENTITIES: [DailySpendEntity; 5] = [
    DailySpendEntity::User,
    DailySpendEntity::Team,
    DailySpendEntity::Org,
    DailySpendEntity::EndUser,
    DailySpendEntity::Agent,
];

#[derive(Debug, thiserror::Error)]
pub enum RedisFlushError {
    #[error("the in-memory spend cohorts could not be published to redis")]
    Publish(#[from] SpendPublishError),
    #[error("the uncommitted spend cohorts could not be restored to redis")]
    Restore(#[from] RepublicationError),
}

// _commit_spend_updates_to_db_with_redis: every pod publishes, only the db_spend_update_job lease holder pops and commits, and the cohorts its commit left uncommitted go back before the release.
pub struct RedisSpendFlush<'run, EntityWriter, Cache, DailyWriter, Window> {
    pub queues: SpendUpdateQueues<'run>,
    pub publisher: &'run RedisSpendPublisher,
    pub buffer: &'run RedisUpdateBuffer,
    pub lease: &'run dyn SpendPodLease,
    pub entity_writer: &'run EntityWriter,
    pub entity_cache: &'run Cache,
    pub policy: SpendRetryPolicy<'run>,
    pub daily_parts: &'run Arc<DailyCommitParts<DailyWriter>>,
    pub window: &'run Window,
}

impl<EntityWriter, Cache, DailyWriter, Window>
    RedisSpendFlush<'_, EntityWriter, Cache, DailyWriter, Window>
where
    EntityWriter: EntityTableWriter,
    Cache: ResetCache,
    DailyWriter: DailyChunkWriter,
    Window: WindowSpendCommit,
{
    pub async fn commit_from_redis(&self, retries: u32) -> Result<(), RedisFlushError> {
        store_spend_updates_in_redis(self.publisher, self.queues).await?;
        let Some(acquired) = self.lease.acquire(DB_SPEND_UPDATE_JOB_NAME).await else {
            return Ok(());
        };
        if !acquired {
            return Ok(());
        }
        let mut uncommitted = UncommittedCohorts::default();
        let outcome = self.drain_and_commit(retries, &mut uncommitted).await;
        if let Err(error) = &outcome {
            tracing::error!(
                "Spend tracking - failed to commit spend updates from Redis to DB. Re-queuing uncommitted transactions to Redis for retry on next tick. Error: {error}"
            );
        }
        // A restore that cannot even encode its cohorts fails here, and the release below is never reached: the source's finally holds the same order.
        restore_transactions_to_redis(self.publisher, uncommitted).await?;
        self.lease.release(DB_SPEND_UPDATE_JOB_NAME).await;
        Ok(())
    }

    // The destructive seven-slot pop, then the source's cohort order; a cohort leaves the uncommitted record only once its commit returned.
    async fn drain_and_commit(
        &self,
        retries: u32,
        uncommitted: &mut UncommittedCohorts,
    ) -> Result<
        (),
        CohortCommitError<EntityWriter::Error, Cache::Failure, DailyWriter::Error, Window::Error>,
    > {
        self.drain_into(uncommitted).await?;
        let policy = SpendRetryPolicy {
            retries,
            ..self.policy
        };
        commit_entity(self.entity_writer, self.entity_cache, policy, uncommitted)
            .await
            .map_err(CohortCommitError::Entity)?;
        for entity in DAILY_ENTITIES {
            commit_daily_slot(self.daily_parts, entity, retries, uncommitted)
                .await
                .map_err(|source| CohortCommitError::Daily { entity, source })?;
        }
        commit_window(self.window, uncommitted)
            .await
            .map_err(CohortCommitError::Window)
    }

    async fn drain_into(
        &self,
        uncommitted: &mut UncommittedCohorts,
    ) -> Result<(), RedisSpendError> {
        let pipeline = self.buffer.drain_pipeline().await?;
        uncommitted.entity = pipeline.entity;
        uncommitted.daily_user = pipeline.daily_user;
        uncommitted.daily_team = pipeline.daily_team;
        uncommitted.daily_org = pipeline.daily_org;
        uncommitted.daily_end_user = pipeline.daily_end_user;
        uncommitted.daily_agent = pipeline.daily_agent;
        uncommitted.window = pipeline.window;
        if let Some(entity) = &uncommitted.entity {
            log_entity_counts(entity);
        }
        Ok(())
    }
}

// The source's pre-commit line, one count per entity table of the popped aggregate.
fn log_entity_counts(transactions: &SpendUpdateTransactions) {
    tracing::info!(
        keys = transactions.key_list_transactions.len(),
        users = transactions.user_list_transactions.len(),
        teams = transactions.team_list_transactions.len(),
        orgs = transactions.org_list_transactions.len(),
        end_users = transactions.end_user_list_transactions.len(),
        team_members = transactions.team_member_list_transactions.len(),
        org_members = transactions.org_member_list_transactions.len(),
        projects = transactions.project_list_transactions.len(),
        tags = transactions.tag_list_transactions.len(),
        agents = transactions.agent_list_transactions.len(),
        model_access_groups = transactions.model_access_group_spend.len(),
        "Spend tracking - committing spend updates from Redis to DB"
    );
}

impl<EntityWriter, Cache, DailyWriter, Window> SpendFlushPass
    for RedisSpendFlush<'_, EntityWriter, Cache, DailyWriter, Window>
where
    EntityWriter: EntityTableWriter,
    Cache: ResetCache,
    DailyWriter: DailyChunkWriter,
    Window: WindowSpendCommit,
{
    type Error = RedisFlushError;

    fn commit_spend_updates(&self, retries: u32) -> ServiceFuture<'_, Result<(), RedisFlushError>> {
        Box::pin(self.commit_from_redis(retries))
    }
}
