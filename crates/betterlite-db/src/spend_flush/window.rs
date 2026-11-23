use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use crate::key_user_writes::retry::WriteClock;
use crate::spend_commit::policy::{SpendFailure, classify_spend_failure};
use crate::window_queue::queue::WindowSpendUpdateQueue;
use crate::window_queue::{WindowQueueError, WindowSpendTransaction};
use crate::window_spend::error::WindowSpendError;
use crate::window_spend::seed::WindowSpendLogs;
use crate::window_spend::store::WindowSpendStore;

pub type WindowCommitFuture<'run, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'run>>;

// _commit_window_spend_updates over the increments a drain aggregated: the caller decides whether a failure can be sent again.
pub trait WindowSpendCommit: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn commit_window_spend<'run>(
        &'run self,
        transactions: &'run [WindowSpendTransaction],
    ) -> WindowCommitFuture<'run, Self::Error>;

    // _spend_commit_failure_is_requeue_safe: a failure the database may have applied, or refused on the data, must not be re-sent.
    fn failure(&self, error: &Self::Error) -> SpendFailure;
}

pub struct PostgresWindowSpend<'run, Logs> {
    pub store: &'run WindowSpendStore,
    pub logs: &'run Logs,
    pub clock: &'run dyn WriteClock,
}

impl<Logs> WindowSpendCommit for PostgresWindowSpend<'_, Logs>
where
    Logs: WindowSpendLogs,
{
    type Error = WindowSpendError;

    fn commit_window_spend<'run>(
        &'run self,
        transactions: &'run [WindowSpendTransaction],
    ) -> WindowCommitFuture<'run, WindowSpendError> {
        let now = self.clock.now();
        Box::pin(
            self.store
                .commit_window_spend_updates(transactions, self.logs, now),
        )
    }

    fn failure(&self, error: &WindowSpendError) -> SpendFailure {
        match error {
            WindowSpendError::Database(source) => classify_spend_failure(source),
            // the deadline covers the COMMIT too, so the upserts may already have applied
            WindowSpendError::TransactionTimeout { .. } => SpendFailure::Ambiguous,
            // a stamp or a seed overflow is refused before the transaction opens, so nothing was written
            WindowSpendError::Stamp(_) | WindowSpendError::Overflow => SpendFailure::Other,
        }
    }
}

// The window stage of the memory flush: rows go back only when the failure is safe to replay, and the discovery drain still runs either way.
pub async fn flush_window_spend_queue<Commit>(
    queue: &WindowSpendUpdateQueue,
    commit: &Commit,
) -> Result<(), WindowQueueError>
where
    Commit: WindowSpendCommit,
{
    let transactions = queue.flush()?;
    if transactions.is_empty() {
        return Ok(());
    }
    let outcome = commit.commit_window_spend(&transactions).await;
    let Err(error) = outcome else {
        return Ok(());
    };
    let increments = transactions.len();
    if commit.failure(&error).is_requeue_safe() {
        tracing::error!(
            "Spend tracking - failed to commit budget window spend updates. Re-queued {increments} window increments for retry on next tick. Error: {error}"
        );
        for transaction in &transactions {
            if let Err(queue_error) = queue.add_update(transaction.clone()).await {
                tracing::error!(
                    "Spend tracking - putting a budget window increment back on the queue failed: {queue_error}"
                );
            }
        }
        return Ok(());
    }
    tracing::error!(
        "Spend tracking - dropped {increments} budget window increments: the failed statement may have applied or the database refused the data, so re-sending it is not safe. Error: {error}"
    );
    Ok(())
}
