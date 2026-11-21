pub mod cutoff;
pub mod deadline;
pub mod deleter;
pub mod settings;
pub mod table;
mod tests;

use settings::{CleanupSettings, DurationParser};
use table::CleanupTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Exhausted,
    BudgetExhausted,
    BatchCapReached,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCleanupResult {
    pub table: CleanupTable,
    pub rows_deleted: u64,
    pub stop_reason: StopReason,
}

// the run that issued a batch counts it, so a cancelled run can say how far it got
pub trait BatchProgress: Send {
    fn record_batch(&mut self, rows_deleted: u64);
}

// SpendLogCleanup: the caller hands over every collaborator LiteLLM reads from proxy globals
pub struct SpendLogCleanup<Clock, Durations, Partitions, Lock> {
    pub settings: CleanupSettings,
    pub clock: Clock,
    pub durations: Durations,
    pub partitions: Partitions,
    pub pod_lock: Option<Lock>,
}

impl<Clock, Durations: DurationParser, Partitions, Lock>
    SpendLogCleanup<Clock, Durations, Partitions, Lock>
{
    // __init__ logs the bounds it starts with; each run reads them again from the settings snapshot
    #[must_use]
    pub fn new(
        settings: CleanupSettings,
        clock: Clock,
        durations: Durations,
        partitions: Partitions,
        pod_lock: Option<Lock>,
    ) -> Self {
        let bounds = settings.bounds(&durations);
        tracing::info!(
            "SpendLogCleanup initialized: batch_size={} max_batches={} run_budget={:?} batch_timeout={:?}",
            bounds.batch_size,
            bounds.max_batches,
            bounds.run_budget,
            bounds.batch_timeout
        );
        Self {
            settings,
            clock,
            durations,
            partitions,
            pod_lock,
        }
    }
}
