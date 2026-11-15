use std::sync::Arc;
use std::time::Duration;

use tokio::time::{Instant, sleep};
use tokio_util::task::TaskTracker;
use tracing::{debug, error, info, warn};

use super::{
    DatabaseFailure, HealthFailureReporter, LifecycleDatabase, LifecycleFuture, ProxyStateSink,
};

pub const HEALTH_CHECK_TRIES: u32 = 3;
pub const HEALTH_CHECK_BUDGET: Duration = Duration::from_secs(10);
pub const FIRST_RETRY_DELAY: Duration = Duration::from_secs(1);
pub const PLANNED_REPLACEMENT_SETTLE: Duration = Duration::from_secs(5);

pub trait HealthTarget: Send + Sync {
    type Error: DatabaseFailure;

    fn generation(&self) -> u64;

    fn probe(&self) -> LifecycleFuture<'_, Result<(), Self::Error>>;

    fn wait_for_replacement(&self, settle: Duration) -> LifecycleFuture<'_, ()>;
}

#[derive(Debug, Clone)]
pub struct HealthFailure {
    pub message: String,
    pub duration: Duration,
}

pub trait HealthCheck {
    type Error: DatabaseFailure;

    fn health_check(&self) -> LifecycleFuture<'_, Result<(), Self::Error>>;

    fn publish_spend_logs_row_count<'borrow>(
        &'borrow self,
        sink: &'borrow dyn ProxyStateSink,
    ) -> LifecycleFuture<'borrow, ()>;
}

pub struct HealthChecker<Database> {
    pub database: Database,
    pub reporter: Arc<dyn HealthFailureReporter>,
    pub tasks: TaskTracker,
}

struct FailedProbe<Target> {
    target: Target,
    generation: u64,
    connection_failure: bool,
    failure: HealthFailure,
}

enum RetryDecision {
    Retry(Duration),
    GiveUp,
}

impl<Database> HealthChecker<Database>
where
    Database: LifecycleDatabase + 'static,
    Database::Target: 'static,
{
    pub async fn run_health_check(&self) -> Result<(), Database::Error> {
        let started = Instant::now();
        let mut attempt = 1;
        loop {
            let attempt_started = Instant::now();
            let target = self.database.probe_target();
            let generation = target.generation();
            let Err(failure) = target.probe().await else {
                return Ok(());
            };
            warn!("LiteLLM Prisma Client Exception health_check(): {failure}");
            let connection_failure = failure.is_connection_failure();
            self.spawn_report(FailedProbe {
                target,
                generation,
                connection_failure,
                failure: HealthFailure {
                    message: failure.to_string(),
                    duration: attempt_started.elapsed(),
                },
            });
            match retry_decision(attempt, started.elapsed()) {
                RetryDecision::GiveUp => return Err(failure),
                RetryDecision::Retry(delay) => {
                    debug!("Backing off... this was attempt #{attempt}");
                    sleep(delay).await;
                }
            }
            attempt = attempt.saturating_add(1);
        }
    }

    #[must_use]
    pub async fn spend_logs_row_count(&self) -> i64 {
        let started = Instant::now();
        let mut attempt = 1;
        loop {
            let failure = match self.database.spend_logs_row_count().await {
                Ok(Some(row_count)) => return row_count,
                Ok(None) => "the query returned no reltuples estimate".to_owned(),
                Err(error) => error.to_string(),
            };
            match retry_decision(attempt, started.elapsed()) {
                RetryDecision::GiveUp => {
                    error!("Error getting LiteLLM_SpendLogs row count: {failure}");
                    return 0;
                }
                RetryDecision::Retry(delay) => {
                    debug!("Backing off... this was attempt #{attempt}");
                    sleep(delay).await;
                }
            }
            attempt = attempt.saturating_add(1);
        }
    }

    fn spawn_report(&self, probe: FailedProbe<Database::Target>) {
        let reporter = Arc::clone(&self.reporter);
        drop(self.tasks.spawn(async move {
            if probe.connection_failure && absorbed(&probe.target, probe.generation).await {
                info!(
                    "Prisma health_check() connection error raced a planned query-engine replacement; not reporting it as a DB exception: {}",
                    probe.failure.message
                );
                return;
            }
            reporter.report(probe.failure).await;
        }));
    }
}

impl<Database> HealthCheck for HealthChecker<Database>
where
    Database: LifecycleDatabase + 'static,
    Database::Target: 'static,
{
    type Error = Database::Error;

    fn health_check(&self) -> LifecycleFuture<'_, Result<(), Self::Error>> {
        Box::pin(self.run_health_check())
    }

    fn publish_spend_logs_row_count<'borrow>(
        &'borrow self,
        sink: &'borrow dyn ProxyStateSink,
    ) -> LifecycleFuture<'borrow, ()> {
        Box::pin(async move { sink.set_spend_logs_row_count(self.spend_logs_row_count().await) })
    }
}

// a probe that failed against a dead engine would report a false DB exception, so a completion is waited for before deciding
async fn absorbed<Target>(target: &Target, generation: u64) -> bool
where
    Target: HealthTarget,
{
    target
        .wait_for_replacement(PLANNED_REPLACEMENT_SETTLE)
        .await;
    if target.generation() == generation {
        return false;
    }
    match target.probe().await {
        Ok(()) => true,
        Err(failure) => {
            debug!("Prisma health_check() confirmation probe failed: {failure}");
            false
        }
    }
}

fn retry_decision(attempt: u32, elapsed: Duration) -> RetryDecision {
    if attempt >= HEALTH_CHECK_TRIES || elapsed >= HEALTH_CHECK_BUDGET {
        return RetryDecision::GiveUp;
    }
    let factor = 2_u32
        .checked_pow(attempt.saturating_sub(1))
        .unwrap_or(u32::MAX);
    let remaining = HEALTH_CHECK_BUDGET.saturating_sub(elapsed);
    RetryDecision::Retry(FIRST_RETRY_DELAY.saturating_mul(factor).min(remaining))
}
