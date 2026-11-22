pub mod partition;

mod tests;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::autorouter_turn_sql::sql::{session_upsert_sql, upsert_autorouter_session_sql};
use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, upsert_params};
use crate::autorouter_turn_sql::write::TurnWriteError;
use crate::daily_spend_sql::bind::execute;
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;
use crate::spend_commit::policy::{SpendFailure, classify_spend_failure};
use crate::tool_spend::{DEFAULT_RETRIES, RetryPause};

use self::partition::{DrainPartition, partition_of, turn_micros};

const JITTER_NANOS: u64 = 1_000_000_000;

pub type TurnFuture<'writer> =
    Pin<Box<dyn Future<Output = Result<(), TurnWriteError>> + Send + 'writer>>;

// write_autorouter_turn with the statement the drain picked: the key and user rollups, or the user-only recovery statement.
pub trait TurnWriter: Send + Sync {
    fn write_turn<'writer>(
        &'writer self,
        turn: &'writer AutoRouterTurnTransaction,
        user_scoped: bool,
    ) -> TurnFuture<'writer>;
}

pub struct PostgresTurns {
    pub store: PostgresStore,
}

impl TurnWriter for PostgresTurns {
    fn write_turn<'writer>(
        &'writer self,
        turn: &'writer AutoRouterTurnTransaction,
        user_scoped: bool,
    ) -> TurnFuture<'writer> {
        Box::pin(async move {
            let statement = if user_scoped {
                session_upsert_sql(true)
            } else {
                upsert_autorouter_session_sql()
            };
            let client = self
                .store
                .session()
                .await
                .map_err(TurnWriteError::Statement)?;
            execute(&client, &statement, &upsert_params(turn))
                .await
                .map_err(|source| TurnWriteError::Statement(backend(source)))?;
            Ok(())
        })
    }
}

pub struct AutoRouterDrain<'run, Writer> {
    pub writer: &'run Writer,
    pub retries: u32,
    pub jitter: &'run dyn JitterSource,
    pub pause: &'run dyn RetryPause,
}

impl<'run, Writer> AutoRouterDrain<'run, Writer> {
    #[must_use]
    pub const fn new(
        writer: &'run Writer,
        jitter: &'run dyn JitterSource,
        pause: &'run dyn RetryPause,
    ) -> Self {
        Self {
            writer,
            retries: DEFAULT_RETRIES,
            jitter,
            pause,
        }
    }
}

impl<Writer> AutoRouterDrain<'_, Writer>
where
    Writer: TurnWriter,
{
    // flush_autorouter_turn_transactions: partition, sort by time inside each partition, and drain sequentially; a stopped partition never blocks another.
    pub async fn flush_turn_transactions(&self, transactions: Vec<AutoRouterTurnTransaction>) {
        if transactions.is_empty() {
            return;
        }
        let mut keyed: Vec<(DrainPartition, AutoRouterTurnTransaction)> = transactions
            .into_iter()
            .map(|turn| (partition_of(&turn), turn))
            .collect();
        keyed.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| turn_micros(left.1.turn_at).cmp(&turn_micros(right.1.turn_at)))
        });
        for group in keyed.chunk_by(|left, right| left.0 == right.0) {
            let turns: Vec<&AutoRouterTurnTransaction> =
                group.iter().map(|(_partition, turn)| turn).collect();
            self.flush_session_partition(&turns).await;
        }
    }

    // _flush_session_partition: after a failed key partition, other users in the failed suffix still advance their user-only histories.
    async fn flush_session_partition(&self, turns: &[&AutoRouterTurnTransaction]) {
        let stopped = self.drain_session_partition(turns, false).await;
        let (_applied, failed_suffix) = turns.split_at(stopped);
        let Some(first) = failed_suffix.first().copied() else {
            return;
        };
        if first.api_key.is_empty() {
            return;
        }
        let failed_user = first.user_id.as_str();
        let mut other_users: Vec<&AutoRouterTurnTransaction> = failed_suffix
            .iter()
            .skip(1)
            .copied()
            .filter(|turn| !turn.user_id.is_empty() && turn.user_id != failed_user)
            .collect();
        other_users.sort_by(|left, right| left.user_id.cmp(&right.user_id));
        for group in other_users.chunk_by(|left, right| left.user_id == right.user_id) {
            self.drain_session_partition(group, true).await;
        }
    }

    // _drain_session_partition: the index of the first turn whose write stopped the partition, or the length when every turn applied.
    async fn drain_session_partition(
        &self,
        turns: &[&AutoRouterTurnTransaction],
        user_scoped: bool,
    ) -> usize {
        for (position, turn) in turns.iter().enumerate() {
            if let Err(error) = self.upsert_turn_with_retry(turn, user_scoped).await {
                tracing::error!(
                    router = %turn.router_name,
                    stopped = turns.len().saturating_sub(position),
                    total = turns.len(),
                    error = %error,
                    "Spend tracking - auto-router session rollup flush failed; the remaining turn writes of this partition are not replayed"
                );
                return position;
            }
        }
        turns.len()
    }

    // _upsert_turn_with_retry: only a proven-unapplied connection error earns another attempt, never a deadlock.
    async fn upsert_turn_with_retry(
        &self,
        turn: &AutoRouterTurnTransaction,
        user_scoped: bool,
    ) -> Result<(), TurnWriteError> {
        let mut attempt = 0_u32;
        loop {
            match self.writer.write_turn(turn, user_scoped).await {
                Ok(()) => return Ok(()),
                Err(error) if !safe_to_replay(&error) || attempt >= self.retries => {
                    return Err(error);
                }
                Err(_retryable) => {
                    self.pause
                        .pause(turn_retry_delay(self.jitter, attempt))
                        .await;
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }
}

fn safe_to_replay(error: &TurnWriteError) -> bool {
    let TurnWriteError::Statement(source) = error;
    let failure = classify_spend_failure(source);
    failure == SpendFailure::NotSent
}

fn turn_retry_delay(jitter: &dyn JitterSource, attempt: u32) -> Duration {
    let base = 1_u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(base)
        .saturating_add(Duration::from_nanos(jitter.uniform_nanos(JITTER_NANOS)))
}
