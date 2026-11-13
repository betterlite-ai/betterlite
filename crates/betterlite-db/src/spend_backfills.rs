use std::future::Future;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use tokio_postgres::{Client, Error as PostgresError};

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
        BackfillPlan::DailyToolSpend { cutover } => {
            run_pair(
                executor,
                daily_tool_spend::statements(cutover),
                |_time_zone, buckets| BackfillOutcome::DailyToolSpend { buckets },
            )
            .await
        }
        BackfillPlan::KeyTotalSpend => {
            run_pair(
                executor,
                key_total_spend::STATEMENTS,
                |active_keys, archived_keys| BackfillOutcome::KeyTotalSpend {
                    active_keys,
                    archived_keys,
                },
            )
            .await
        }
        BackfillPlan::KeyTotalSpendFromLogs => {
            run_pair(
                executor,
                key_total_spend_from_logs::STATEMENTS,
                |active_keys, archived_keys| BackfillOutcome::KeyTotalSpendFromLogs {
                    active_keys,
                    archived_keys,
                },
            )
            .await
        }
    }
}

async fn run_pair<Executor>(
    executor: &Executor,
    statements: [(BackfillStatement, &'static str); 2],
    outcome: fn(u64, u64) -> BackfillOutcome,
) -> Result<BackfillOutcome, BackfillError>
where
    Executor: ExecutesBackfillStatements + Sync,
{
    let [(first, first_failure), (second, second_failure)] = statements;
    let first_rows =
        executor
            .execute_backfill(first)
            .await
            .map_err(|source| BackfillError::Statement {
                statement: first_failure,
                source,
            })?;
    let second_rows =
        executor
            .execute_backfill(second)
            .await
            .map_err(|source| BackfillError::Statement {
                statement: second_failure,
                source,
            })?;
    Ok(outcome(first_rows, second_rows))
}
