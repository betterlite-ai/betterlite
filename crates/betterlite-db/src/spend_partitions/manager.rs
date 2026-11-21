use chrono::{DateTime, Utc};
use tokio_postgres::Client;

use super::calendar::{PartitionClock, PartitionInterval, upcoming_partitions};
use super::catalog::{
    BoundedTransactionError, RemainingBudget, create_partition_statement, drop_partition_statement,
    execute_ddl, list_partitions, within_deadline,
};
use super::partition_bound::{PartitionBoundError, select_partitions_to_drop};

pub struct SpendPartitionManager<Clock: PartitionClock> {
    pub interval: PartitionInterval,
    pub precreate_ahead: i64,
    pub clock: Clock,
}

#[derive(Debug, thiserror::Error)]
pub enum PartitionMaintenanceError {
    #[error("could not list the spend-log partitions")]
    List {
        #[source]
        source: BoundedTransactionError,
    },
    #[error(transparent)]
    Bound(#[from] PartitionBoundError),
}

impl<Clock: PartitionClock> SpendPartitionManager<Clock> {
    #[must_use]
    pub const fn new(interval: PartitionInterval, precreate_ahead: i64, clock: Clock) -> Self {
        Self {
            interval,
            precreate_ahead,
            clock,
        }
    }

    // ensure_partitions: a fresh budget before each CREATE, and a failed one is left for the next run
    pub async fn ensure_partitions(
        &self,
        client: &mut Client,
        remaining: RemainingBudget<'_>,
    ) -> Vec<String> {
        let mut ensured = Vec::new();
        let planned = upcoming_partitions(self.clock.today(), self.interval, self.precreate_ahead);
        for partition in planned {
            let Some(timeout_ms) = remaining() else {
                tracing::info!(
                    "Run budget spent, leaving the remaining partitions for the next run"
                );
                break;
            };
            let statement = create_partition_statement(&partition);
            let created =
                within_deadline(timeout_ms, execute_ddl(client, &statement, timeout_ms)).await;
            match created {
                Ok(()) => ensured.push(partition.name),
                Err(error) => {
                    tracing::warn!(
                        "Failed to ensure spend-log partition {}: {error}",
                        partition.name
                    );
                }
            }
        }
        ensured
    }
}

// drop_partitions_older_than: a fresh budget before the list and before each DROP
pub async fn drop_partitions_older_than(
    client: &mut Client,
    cutoff: DateTime<Utc>,
    remaining: RemainingBudget<'_>,
) -> Result<Vec<String>, PartitionMaintenanceError> {
    let Some(list_timeout_ms) = remaining() else {
        return Ok(Vec::new());
    };
    let partitions = within_deadline(list_timeout_ms, list_partitions(client, list_timeout_ms))
        .await
        .map_err(|source| PartitionMaintenanceError::List { source })?;
    let to_drop = select_partitions_to_drop(&partitions, cutoff.naive_utc())?;
    let mut dropped = Vec::new();
    for name in to_drop {
        let Some(timeout_ms) = remaining() else {
            tracing::info!("Run budget spent, leaving the remaining partitions for the next run");
            break;
        };
        let statement = drop_partition_statement(&name);
        let dropped_partition =
            within_deadline(timeout_ms, execute_ddl(client, &statement, timeout_ms)).await;
        match dropped_partition {
            Ok(()) => dropped.push(name),
            Err(error) => tracing::warn!("Failed to drop spend-log partition {name}: {error}"),
        }
    }
    Ok(dropped)
}
