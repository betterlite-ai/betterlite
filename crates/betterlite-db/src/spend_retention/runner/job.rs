use std::sync::Arc;

use betterlite_prometheus::collector_registry::CollectorRegistry;

use crate::cleanup_metrics::CleanupMetrics;
use crate::postgres_store::session::PostgresStore;
use crate::spend_retention::batches::SpendLogCleanup;
use crate::spend_retention::batches::settings::DurationParser;

use super::clock::CleanupClock;
use super::error::RunError;
use super::lock::CleanupLock;
use super::partitions::PartitionMaintenance;

// the owned handle B8371's startup and reload scheduling reuse, so no second retention job is registered here
pub struct SpendLogCleanupJob<Clock, Durations, Partitions, Lock> {
    cleanup: Arc<SpendLogCleanup<Clock, Durations, Partitions, Lock>>,
    store: PostgresStore,
    registry: Arc<CollectorRegistry>,
}

impl<Clock, Durations, Partitions, Lock> SpendLogCleanupJob<Clock, Durations, Partitions, Lock>
where
    Clock: CleanupClock,
    Durations: DurationParser,
    Partitions: PartitionMaintenance,
    Lock: CleanupLock,
{
    #[must_use]
    pub fn new(
        cleanup: Arc<SpendLogCleanup<Clock, Durations, Partitions, Lock>>,
        store: PostgresStore,
        registry: Arc<CollectorRegistry>,
    ) -> Self {
        Self {
            cleanup,
            store,
            registry,
        }
    }

    // one pooled session per run, the same client the source receives as prisma_client
    pub async fn run(&self) -> Result<(), RunError> {
        let metrics = CleanupMetrics::new(&self.registry);
        let mut client = self.store.session().await?;
        self.cleanup
            .cleanup_old_spend_logs(&mut client, &metrics)
            .await
    }
}
