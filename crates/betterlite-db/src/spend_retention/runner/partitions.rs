use std::future::Future;

use chrono::{DateTime, Utc};
use tokio_postgres::Client;

use crate::spend_partitions::calendar::PartitionClock;
use crate::spend_partitions::catalog::{RemainingBudget, is_partitioned as probe_partitioned};
use crate::spend_partitions::manager::{
    PartitionMaintenanceError, SpendPartitionManager, drop_partitions_older_than,
};

// B1739's partition maintenance behind a seam, so a run's partition gate can be tested without a partitioned table
pub trait PartitionMaintenance: Send + Sync {
    fn is_partitioned(
        &self,
        client: &mut Client,
        remaining: RemainingBudget<'_>,
    ) -> impl Future<Output = bool> + Send;

    fn ensure_upcoming(
        &self,
        client: &mut Client,
        remaining: RemainingBudget<'_>,
    ) -> impl Future<Output = ()> + Send;

    fn drop_expired(
        &self,
        client: &mut Client,
        cutoff: DateTime<Utc>,
        remaining: RemainingBudget<'_>,
    ) -> impl Future<Output = Result<Vec<String>, PartitionMaintenanceError>> + Send;
}

impl<Clock: PartitionClock> PartitionMaintenance for SpendPartitionManager<Clock> {
    async fn is_partitioned(&self, client: &mut Client, remaining: RemainingBudget<'_>) -> bool {
        probe_partitioned(client, remaining).await
    }

    async fn ensure_upcoming(&self, client: &mut Client, remaining: RemainingBudget<'_>) {
        let ensured = Self::ensure_partitions(self, client, remaining).await;
        drop(ensured);
    }

    async fn drop_expired(
        &self,
        client: &mut Client,
        cutoff: DateTime<Utc>,
        remaining: RemainingBudget<'_>,
    ) -> Result<Vec<String>, PartitionMaintenanceError> {
        drop_partitions_older_than(client, cutoff, remaining).await
    }
}
