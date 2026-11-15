use std::sync::Arc;
use std::time::Duration;

use tokio::time::{sleep, timeout};
use tokio_util::task::TaskTracker;
use tracing::{debug, info};

use super::health::HealthTarget;
use super::read_only::{ReadOnlyWriter, ReconnectControl};
use super::{DatabaseFailure, LifecycleDatabase, TrackedTask};

pub const WRITER_UNAVAILABLE_REASON: &str = "db_health_watchdog_writer_unavailable";
pub const WRITER_READ_ONLY_REASON: &str = "db_health_watchdog_writer_read_only";
pub const CONNECTION_ERROR_REASON: &str = "db_health_watchdog_connection_error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogSettings {
    pub enabled: bool,
    pub interval: Duration,
    pub probe_timeout: Duration,
    pub reconnect_timeout: Duration,
}

pub struct Watchdog<Database, Reconnect> {
    pub database: Database,
    pub reconnect: Arc<Reconnect>,
    pub read_only: Arc<ReadOnlyWriter<Reconnect>>,
    pub settings: WatchdogSettings,
    pub task: TrackedTask,
}

enum ProbeOutcome {
    Answered,
    Failed,
}

enum WriterProbe {
    ReadOnly,
    Writable,
    Failed,
}

impl<Database, Reconnect> Watchdog<Database, Reconnect>
where
    Database: LifecycleDatabase,
    Reconnect: ReconnectControl,
{
    pub fn start(self: &Arc<Self>, tasks: &TaskTracker)
    where
        Self: Send + Sync + 'static,
    {
        if !self.settings.enabled {
            debug!("Prisma DB health watchdog disabled via PRISMA_HEALTH_WATCHDOG_ENABLED");
            return;
        }
        let watchdog = Arc::clone(self);
        if !self.task.start_with(tasks, async move {
            watchdog.run().await;
        }) {
            return;
        }
        info!(
            "Started Prisma DB health watchdog (interval={}s, reconnect_cooldown={}s, probe_timeout={}s, reconnect_timeout={}s)",
            self.settings.interval.as_secs_f64(),
            self.read_only.cooldown.as_secs_f64(),
            self.settings.probe_timeout.as_secs_f64(),
            self.settings.reconnect_timeout.as_secs_f64()
        );
    }

    pub async fn stop(&self) {
        if self.task.stop().await {
            info!("Stopped Prisma DB health watchdog");
        }
    }

    async fn run(&self) {
        loop {
            sleep(self.settings.interval).await;
            self.turn().await;
        }
    }

    pub(crate) async fn turn(&self) {
        if matches!(self.routed_probe().await, ProbeOutcome::Failed) {
            return;
        }
        if self.database.writer_unavailable() {
            self.attempt_reconnect(WRITER_UNAVAILABLE_REASON).await;
            return;
        }
        match self.writer_probe().await {
            WriterProbe::ReadOnly => {
                let _recreated = self
                    .read_only
                    .recreate(
                        WRITER_READ_ONLY_REASON,
                        Some(self.settings.reconnect_timeout),
                    )
                    .await;
            }
            WriterProbe::Writable => self.read_only.reset(),
            WriterProbe::Failed => {}
        }
    }

    async fn routed_probe(&self) -> ProbeOutcome {
        let target = self.database.probe_target();
        match timeout(self.settings.probe_timeout, target.probe()).await {
            Ok(Ok(())) => ProbeOutcome::Answered,
            Ok(Err(failure)) => {
                self.handle_probe_failure(&failure).await;
                ProbeOutcome::Failed
            }
            Err(_elapsed) => {
                self.attempt_reconnect(CONNECTION_ERROR_REASON).await;
                ProbeOutcome::Failed
            }
        }
    }

    async fn writer_probe(&self) -> WriterProbe {
        match timeout(
            self.settings.probe_timeout,
            self.database.writer_is_read_only(),
        )
        .await
        {
            Ok(Ok(true)) => WriterProbe::ReadOnly,
            Ok(Ok(false)) => WriterProbe::Writable,
            Ok(Err(failure)) => {
                self.handle_probe_failure(&failure).await;
                WriterProbe::Failed
            }
            Err(_elapsed) => {
                self.attempt_reconnect(CONNECTION_ERROR_REASON).await;
                WriterProbe::Failed
            }
        }
    }

    async fn handle_probe_failure(&self, failure: &Database::Error) {
        if failure.is_infrastructure_failure() {
            self.attempt_reconnect(CONNECTION_ERROR_REASON).await;
        } else {
            debug!("Prisma DB health watchdog observed non-DB error: {failure}");
        }
    }

    async fn attempt_reconnect(&self, reason: &str) {
        let _reconnected = self
            .reconnect
            .reconnect(reason, Some(self.settings.reconnect_timeout))
            .await;
    }
}
