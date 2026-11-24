use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::sync::Arc;

use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::daily_spend_sql::entities::DailySpendEntity;
use crate::entity_spend_commit::EntityTableWriter;
use crate::entity_spend_commit::progress::{EntityCommitError, EntitySpendCommit};
use crate::redis_spend::buffer::RedisSpendError;
use crate::redis_spend::republication::UncommittedCohorts;
use crate::spend_commit::daily::DailyChunkWriter;
use crate::spend_commit::phase::CommitPhase;
use crate::spend_commit::policy::{SpendRetryPolicy, SpendUpdateFailure};
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions, spend_tables_left_to_send};
use crate::spend_flush::daily::DailyCommitParts;
use crate::spend_flush::memory::spend_table_transactions;
use crate::spend_flush::redis::commit_daily;
use crate::spend_flush::window::WindowSpendCommit;
use crate::spend_queues::daily::DailySpendUpdate;
use crate::spend_queues::entity::{SpendAmount, SpendUpdateTransactions};

// Why the holder's commit stopped, with every cohort it did not finish still in the uncommitted record.
#[derive(Debug, thiserror::Error)]
pub(super) enum CohortCommitError<EntityFailure, CacheFailure, DailyFailure, WindowFailure>
where
    EntityFailure: SpendUpdateFailure,
    CacheFailure: StdError + Send + Sync + 'static,
    DailyFailure: StdError + Send + Sync + 'static,
    WindowFailure: StdError + Send + Sync + 'static,
{
    #[error("the redis spend buffer could not be drained")]
    Drain(#[from] RedisSpendError),
    #[error("the entity spend commit failed")]
    Entity(#[source] EntityCommitError<EntityFailure, CacheFailure>),
    #[error("the daily {entity:?} spend commit failed")]
    Daily {
        entity: DailySpendEntity,
        #[source]
        source: DailyFailure,
    },
    #[error("the budget window spend commit failed")]
    Window(#[source] WindowFailure),
}

// _commit_spend_updates_to_db over a popped entity aggregate: the tables it did not reach travel back as the projection a later tick decodes.
pub(super) async fn commit_entity<Writer, Cache>(
    writer: &Writer,
    cache: &Cache,
    policy: SpendRetryPolicy<'_>,
    uncommitted: &mut UncommittedCohorts,
) -> Result<(), EntityCommitError<Writer::Error, Cache::Failure>>
where
    Writer: EntityTableWriter,
    Cache: ResetCache,
{
    let Some(transactions) = uncommitted.entity.take() else {
        return Ok(());
    };
    let tables = spend_table_transactions(&transactions);
    let commit = EntitySpendCommit {
        writer,
        policy,
        cache,
    };
    let mut committed = Vec::new();
    let outcome = commit
        .commit_spend_updates(&tables, |table| committed.push(table))
        .await;
    let Err(error) = outcome else {
        return Ok(());
    };
    uncommitted.entity = spend_tables_left_to_send(&tables, &committed, error.spend_failure())
        .map(|remaining| spend_update_transactions(&remaining));
    Err(error)
}

// _update_daily_*_spend over one popped cohort: the written map keeps the keys the commit could not settle, so the slot holds the safe remainder again.
pub(super) async fn commit_daily_slot<Writer>(
    parts: &DailyCommitParts<Writer>,
    entity: DailySpendEntity,
    retries: u32,
    uncommitted: &mut UncommittedCohorts,
) -> Result<(), Writer::Error>
where
    Writer: DailyChunkWriter,
{
    let Some(slot) = daily_slot(uncommitted, entity) else {
        return Ok(());
    };
    let Some(mut pending) = slot.take() else {
        return Ok(());
    };
    let phase = Arc::new(CommitPhase::new());
    let outcome = commit_daily(parts, retries, phase, entity, &mut pending).await;
    if outcome.is_err() {
        *slot = Some(pending);
    }
    outcome
}

const fn daily_slot(
    uncommitted: &mut UncommittedCohorts,
    entity: DailySpendEntity,
) -> Option<&mut Option<DailySpendUpdate>> {
    match entity {
        DailySpendEntity::User => Some(&mut uncommitted.daily_user),
        DailySpendEntity::Team => Some(&mut uncommitted.daily_team),
        DailySpendEntity::Org => Some(&mut uncommitted.daily_org),
        DailySpendEntity::EndUser => Some(&mut uncommitted.daily_end_user),
        DailySpendEntity::Agent => Some(&mut uncommitted.daily_agent),
        DailySpendEntity::Tag => None,
    }
}

// _commit_window_spend_updates: a replay-safe failure keeps the cohort for the restore, and an unsafe one drops it with the source's error.
pub(super) async fn commit_window<Commit>(
    commit: &Commit,
    uncommitted: &mut UncommittedCohorts,
) -> Result<(), Commit::Error>
where
    Commit: WindowSpendCommit,
{
    let Some(transactions) = uncommitted.window.take() else {
        return Ok(());
    };
    let outcome = commit.commit_window_spend(&transactions).await;
    let Err(error) = outcome else {
        return Ok(());
    };
    if commit.failure(&error).is_requeue_safe() {
        uncommitted.window = Some(transactions);
    } else {
        tracing::error!(
            "Spend tracking - dropped {} budget window increments: the failed statement may have applied or the database refused the data, so re-sending it is not safe. Error: {error}",
            transactions.len()
        );
    }
    Err(error)
}

// _spend_tables_left_to_send back into the wire batch: a committed or dropped table leaves its bucket empty, exactly as the source's null does.
fn spend_update_transactions(tables: &SpendTableTransactions) -> SpendUpdateTransactions {
    let mut transactions = SpendUpdateTransactions::default();
    for (table, increments) in tables {
        let Some(rows) = increments else {
            continue;
        };
        let bucket = bucket_mut(&mut transactions, *table);
        for (entity_id, amount) in rows {
            bucket.insert(entity_id.clone(), SpendAmount(*amount));
        }
    }
    transactions
}

const fn bucket_mut(
    transactions: &mut SpendUpdateTransactions,
    table: SpendTable,
) -> &mut BTreeMap<String, SpendAmount> {
    match table {
        SpendTable::UserList => &mut transactions.user_list_transactions,
        SpendTable::EndUserList => &mut transactions.end_user_list_transactions,
        SpendTable::KeyList => &mut transactions.key_list_transactions,
        SpendTable::TeamList => &mut transactions.team_list_transactions,
        SpendTable::TeamMemberList => &mut transactions.team_member_list_transactions,
        SpendTable::OrgList => &mut transactions.org_list_transactions,
        SpendTable::OrgMemberList => &mut transactions.org_member_list_transactions,
        SpendTable::ProjectList => &mut transactions.project_list_transactions,
        SpendTable::TagList => &mut transactions.tag_list_transactions,
        SpendTable::ModelAccessGroupList => &mut transactions.model_access_group_spend,
        SpendTable::AgentList => &mut transactions.agent_list_transactions,
    }
}
