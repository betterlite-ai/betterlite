use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use deadpool_postgres::Client;
use tokio_postgres::Error as PostgresError;

pub mod daily_tool_spend;
pub mod fixtures;
pub mod key_total_spend;
pub mod key_total_spend_from_logs;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillStatement {
    SessionTimeZoneUtc,
    DailyToolSpend { cutover: DateTime<Utc> },
    FloorActiveKeys,
    FloorArchivedKeys,
    HistoryActiveKeys,
    HistoryArchivedKeys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillPlan {
    DailyToolSpend { cutover: DateTime<Utc> },
    KeyTotalSpend,
    KeyTotalSpendFromLogs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillOutcome {
    DailyToolSpend {
        buckets: u64,
    },
    KeyTotalSpend {
        active_keys: u64,
        archived_keys: u64,
    },
    KeyTotalSpendFromLogs {
        active_keys: u64,
        archived_keys: u64,
    },
}

pub type BackfillFuture<'executor> =
    Pin<Box<dyn Future<Output = Result<u64, PostgresError>> + Send + 'executor>>;

pub trait ExecutesBackfillStatements {
    fn execute_backfill(&self, statement: BackfillStatement) -> BackfillFuture<'_>;
}

impl ExecutesBackfillStatements for Client {
    fn execute_backfill(&self, statement: BackfillStatement) -> BackfillFuture<'_> {
        Box::pin(async move {
            match statement {
                BackfillStatement::SessionTimeZoneUtc => {
                    self.execute(daily_tool_spend::SET_TIME_ZONE_SQL, &[]).await
                }
                BackfillStatement::DailyToolSpend { cutover } => {
                    self.execute(daily_tool_spend::INSERT_SQL, &[&cutover])
                        .await
                }
                BackfillStatement::FloorActiveKeys => {
                    self.execute(key_total_spend::ACTIVE_SQL, &[]).await
                }
                BackfillStatement::FloorArchivedKeys => {
                    self.execute(key_total_spend::ARCHIVED_SQL, &[]).await
                }
                BackfillStatement::HistoryActiveKeys => {
                    self.execute(key_total_spend_from_logs::ACTIVE_SQL, &[])
                        .await
                }
                BackfillStatement::HistoryArchivedKeys => {
                    self.execute(key_total_spend_from_logs::ARCHIVED_SQL, &[])
                        .await
                }
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackfillError {
    #[error("{statement} failed: {source}{}", .source.as_db_error().map_or_else(String::new, |server| format!(": {server}")))]
    Statement {
        statement: &'static str,
        #[source]
        source: PostgresError,
    },
}

pub async fn run_plan<Executor>(
    executor: &Executor,
    plan: BackfillPlan,
) -> Result<BackfillOutcome, BackfillError>
where
    Executor: ExecutesBackfillStatements + Sync,
{
    match plan {
        BackfillPlan::DailyToolSpend { cutover } => run_daily(executor, cutover).await,
        BackfillPlan::KeyTotalSpend => run_floor(executor).await,
        BackfillPlan::KeyTotalSpendFromLogs => run_history(executor).await,
    }
}

async fn run_daily<Executor>(
    executor: &Executor,
    cutover: DateTime<Utc>,
) -> Result<BackfillOutcome, BackfillError>
where
    Executor: ExecutesBackfillStatements + Sync,
{
    executor
        .execute_backfill(BackfillStatement::SessionTimeZoneUtc)
        .await
        .map_err(|source| BackfillError::Statement {
            statement: daily_tool_spend::SESSION_TIME_ZONE_FAILURE,
            source,
        })?;
    let buckets = executor
        .execute_backfill(BackfillStatement::DailyToolSpend { cutover })
        .await
        .map_err(|source| BackfillError::Statement {
            statement: daily_tool_spend::INSERT_FAILURE,
            source,
        })?;
    Ok(BackfillOutcome::DailyToolSpend { buckets })
}

async fn run_floor<Executor>(executor: &Executor) -> Result<BackfillOutcome, BackfillError>
where
    Executor: ExecutesBackfillStatements + Sync,
{
    let active_keys = executor
        .execute_backfill(BackfillStatement::FloorActiveKeys)
        .await
        .map_err(|source| BackfillError::Statement {
            statement: key_total_spend::ACTIVE_FAILURE,
            source,
        })?;
    let archived_keys = executor
        .execute_backfill(BackfillStatement::FloorArchivedKeys)
        .await
        .map_err(|source| BackfillError::Statement {
            statement: key_total_spend::ARCHIVED_FAILURE,
            source,
        })?;
    Ok(BackfillOutcome::KeyTotalSpend {
        active_keys,
        archived_keys,
    })
}

async fn run_history<Executor>(executor: &Executor) -> Result<BackfillOutcome, BackfillError>
where
    Executor: ExecutesBackfillStatements + Sync,
{
    let active_keys = executor
        .execute_backfill(BackfillStatement::HistoryActiveKeys)
        .await
        .map_err(|source| BackfillError::Statement {
            statement: key_total_spend_from_logs::ACTIVE_FAILURE,
            source,
        })?;
    let archived_keys = executor
        .execute_backfill(BackfillStatement::HistoryArchivedKeys)
        .await
        .map_err(|source| BackfillError::Statement {
            statement: key_total_spend_from_logs::ARCHIVED_FAILURE,
            source,
        })?;
    Ok(BackfillOutcome::KeyTotalSpendFromLogs {
        active_keys,
        archived_keys,
    })
}
