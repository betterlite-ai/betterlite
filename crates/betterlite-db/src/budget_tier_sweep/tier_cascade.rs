use std::convert::Infallible;

use crate::budget_cascade::BudgetCascade;
use crate::budget_cascade::cache_invalidation::commit_and_invalidate_cascade;
use crate::budget_cascade::collect_cascade::collect_budget_cascade;
use crate::budget_cascade::linked_rows::PostgresLinkedRows;
use crate::budget_permission_rows::LiteLLMBudgetTableFull;
use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::budget_reset_policy::entity_reset::ResetSchedule;
use crate::budget_reset_policy::retry::RetryMode;
use crate::budget_reset_policy::rollover::RolloverPolicy;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::identity_budget_reset::faults::write_fault;
use crate::postgres_store::session::PostgresStore;
use crate::spend_reset_unit_of_work::UnitOfWorkError;

pub(crate) const CASCADE_WRITE_REASON: &str = "reset_budget_write_budget_cascade_failure";

#[derive(Debug, Clone, Copy)]
pub(crate) struct TierCascadeRequest<'request, Schedule> {
    pub(crate) budgets: &'request [LiteLLMBudgetTableFull],
    pub(crate) rollover: RolloverPolicy,
    pub(crate) schedule: &'request Schedule,
    pub(crate) default_enduser_budget_id: Option<&'request str>,
}

// A failed chunk names how many tiers it read, so the failure event counts a commit failure's tiers and a collect failure's none.
pub(crate) enum TierCascadeFailure<ScheduleFailure> {
    Collect(ScheduleFailure),
    Commit { budgets: usize, error: DbError },
}

// _reset_expired_budget_cascade's collect and commit: a collect failure leaves no cascade, and a commit failure leaves the tiers due for the next run.
pub(crate) async fn run_tier_cascade<Schedule, Cache, Reconnect>(
    store: &PostgresStore,
    cache: &Cache,
    request: TierCascadeRequest<'_, Schedule>,
    reconnect: Option<&mut Reconnect>,
) -> Result<BudgetCascade, TierCascadeFailure<Schedule::Failure>>
where
    Schedule: ResetSchedule + Sync,
    Cache: ResetCache,
    Reconnect: DbReconnect + Send,
{
    let source = PostgresLinkedRows {
        store: store.clone(),
    };
    let cascade =
        collect_budget_cascade(&source, request.budgets, request.rollover, request.schedule)
            .await
            .map_err(TierCascadeFailure::Collect)?;
    let budgets = cascade.budgets.len();
    let committed = commit_cascade(
        store,
        cache,
        &cascade,
        request.default_enduser_budget_id,
        reconnect,
    )
    .await;
    match committed {
        Ok(()) => Ok(cascade),
        Err(error) => Err(TierCascadeFailure::Commit { budgets, error }),
    }
}

pub(crate) async fn commit_cascade<Cache, Reconnect>(
    store: &PostgresStore,
    cache: &Cache,
    cascade: &BudgetCascade,
    default_enduser_budget_id: Option<&str>,
    reconnect: Option<&mut Reconnect>,
) -> Result<(), DbError>
where
    Cache: ResetCache,
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Write.policy(CASCADE_WRITE_REASON);
    let attempt = move || commit_cascade_once(store, cache, cascade, default_enduser_budget_id);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

async fn commit_cascade_once<Cache>(
    store: &PostgresStore,
    cache: &Cache,
    cascade: &BudgetCascade,
    default_enduser_budget_id: Option<&str>,
) -> Result<(), DbError>
where
    Cache: ResetCache,
{
    commit_and_invalidate_cascade(store, cache, cascade, default_enduser_budget_id)
        .await
        .map_err(|error| write_fault(UnitOfWorkError::<Infallible>::Commit(error)))
}

mod tests;
