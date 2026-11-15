use std::future::Future;
use std::pin::Pin;

use tokio_postgres::{Client, Error as PostgresError};

pub mod fixtures;
pub mod partition;
mod tests;
pub mod unpartition;

const BEGIN_SQL: &str = "BEGIN";
const COMMIT_SQL: &str = "COMMIT";
const ROLLBACK_SQL: &str = "ROLLBACK";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendPartitionOperation {
    Partition,
    Unpartition,
}

impl SpendPartitionOperation {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Partition => "partition-spend-logs",
            Self::Unpartition => "unpartition-spend-logs",
        }
    }

    #[must_use]
    pub const fn steps(self) -> &'static [SpendPartitionStep] {
        match self {
            Self::Partition => partition::STEPS,
            Self::Unpartition => unpartition::STEPS,
        }
    }

    #[must_use]
    pub fn script(self) -> String {
        let mut script = String::new();
        for step in self.steps() {
            script.push_str(step.sql);
            script.push_str(";\n");
        }
        script
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendPartitionStep {
    pub label: &'static str,
    pub sql: &'static str,
}

const ROLLBACK_STEP: SpendPartitionStep = SpendPartitionStep {
    label: "rolling back the transaction",
    sql: ROLLBACK_SQL,
};

pub type SpendPartitionFuture<'executor> =
    Pin<Box<dyn Future<Output = Result<(), PostgresError>> + Send + 'executor>>;

pub trait ExecutesSpendPartitionSteps {
    fn execute_spend_partition_step(&self, step: SpendPartitionStep) -> SpendPartitionFuture<'_>;
}

impl ExecutesSpendPartitionSteps for Client {
    fn execute_spend_partition_step(&self, step: SpendPartitionStep) -> SpendPartitionFuture<'_> {
        // the simple protocol runs each statement exactly as psql does, and ROLLBACK still parses inside an aborted block
        Box::pin(async move { self.batch_execute(step.sql).await })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SpendPartitionError {
    #[error("{} failed while {step}: {source}{}", .operation.name(), .source.as_db_error().map_or_else(String::new, |server| format!(": {server}")))]
    Statement {
        operation: SpendPartitionOperation,
        step: &'static str,
        #[source]
        source: PostgresError,
    },
    #[error("{} failed while {step}: {source}{}; the rollback also failed: {rollback}{}", .operation.name(), .source.as_db_error().map_or_else(String::new, |server| format!(": {server}")), .rollback.as_db_error().map_or_else(String::new, |server| format!(": {server}")))]
    Rollback {
        operation: SpendPartitionOperation,
        step: &'static str,
        #[source]
        source: PostgresError,
        rollback: PostgresError,
    },
}

pub async fn run_conversion<Executor>(
    executor: &Executor,
    operation: SpendPartitionOperation,
) -> Result<(), SpendPartitionError>
where
    Executor: ExecutesSpendPartitionSteps + Sync,
{
    for step in operation.steps() {
        if let Err(source) = executor.execute_spend_partition_step(*step).await {
            return Err(roll_back(executor, operation, *step, source).await);
        }
    }
    Ok(())
}

async fn roll_back<Executor>(
    executor: &Executor,
    operation: SpendPartitionOperation,
    step: SpendPartitionStep,
    source: PostgresError,
) -> SpendPartitionError
where
    Executor: ExecutesSpendPartitionSteps + Sync,
{
    let undo = executor.execute_spend_partition_step(ROLLBACK_STEP).await;
    if let Err(failure) = undo {
        return SpendPartitionError::Rollback {
            operation,
            step: step.label,
            source,
            rollback: failure,
        };
    }
    SpendPartitionError::Statement {
        operation,
        step: step.label,
        source,
    }
}
