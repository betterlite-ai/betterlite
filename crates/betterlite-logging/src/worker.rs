use std::error::Error as StdError;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use thiserror::Error;
use tokio::pin;
use tokio::runtime::{Handle, TryCurrentError};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::Span;

use counters::{Counters, shutdown_report};
use timeouts::{TimeoutBurst, summarize};

mod counters;
mod runner;
mod timeouts;

mod tests;

pub const DEFAULT_MAX_QUEUE_SIZE: usize = 50_000;
pub const DEFAULT_CONCURRENCY: usize = 100;
pub const DEFAULT_EVENT_TIMEOUT: Duration = Duration::from_secs(20);
pub const DEFAULT_SHUTDOWN_DRAIN: Duration = Duration::from_secs(30);
pub const DEFAULT_TIMEOUT_SUMMARY_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerSettings {
    pub max_queue_size: usize,
    pub concurrency: usize,
    pub event_timeout: Duration,
    pub shutdown_drain: Duration,
    pub timeout_summary_window: Duration,
}

impl Default for WorkerSettings {
    fn default() -> Self {
        Self {
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
            concurrency: DEFAULT_CONCURRENCY,
            event_timeout: DEFAULT_EVENT_TIMEOUT,
            shutdown_drain: DEFAULT_SHUTDOWN_DRAIN,
            timeout_summary_window: DEFAULT_TIMEOUT_SUMMARY_WINDOW,
        }
    }
}

pub trait Callback: Send + 'static {
    type Error: StdError + Send + 'static;

    fn name(&self) -> &str;

    fn call(self) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Accepted,
    Rejected(Rejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    QueueFull,
    Closed,
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("the logging worker queue size must be greater than zero")]
    ZeroQueueSize,
    #[error("the logging worker concurrency must be greater than zero")]
    ZeroConcurrency,
    #[error("the logging worker must start inside a tokio runtime")]
    NoRuntime(#[source] TryCurrentError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerStats {
    pub accepted: usize,
    pub rejected_full: usize,
    pub rejected_closed: usize,
    pub dequeued: usize,
    pub started: usize,
    pub completed: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub cancelled: usize,
    pub pending: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushReport {
    pub accepted: usize,
    pub completed: usize,
    pub failed: usize,
    pub timed_out: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub accepted: usize,
    pub completed: usize,
    pub failed: usize,
    pub timed_out: usize,
    pub cancelled: usize,
    pub dropped: usize,
    pub drained: bool,
}

struct Queued<C> {
    span: Span,
    callback: C,
}

pub struct LoggingWorker<C>
where
    C: Callback,
{
    settings: WorkerSettings,
    admission: Mutex<Option<mpsc::Sender<Queued<C>>>>,
    counters: Arc<Counters>,
    settled: Arc<Notify>,
    shutdown: CancellationToken,
    runner: Mutex<Option<JoinHandle<()>>>,
}

impl<C> LoggingWorker<C>
where
    C: Callback,
{
    pub fn start(settings: WorkerSettings) -> Result<Self, WorkerError> {
        if settings.max_queue_size == 0 {
            return Err(WorkerError::ZeroQueueSize);
        }
        if settings.concurrency == 0 {
            return Err(WorkerError::ZeroConcurrency);
        }
        let runtime = Handle::try_current().map_err(WorkerError::NoRuntime)?;
        let (admission, receiver) = mpsc::channel(settings.max_queue_size);
        let counters = Arc::new(Counters::default());
        let settled = Arc::new(Notify::new());
        let burst = Arc::new(TimeoutBurst::default());
        let shutdown = CancellationToken::new();
        let summarizer = runtime.spawn(summarize(
            Arc::clone(&burst),
            Arc::clone(&counters),
            settings.timeout_summary_window,
            settings.event_timeout,
            shutdown.clone(),
        ));
        let runner = runtime.spawn(runner::run(
            receiver,
            settings,
            runner::RunnerContext {
                counters: Arc::clone(&counters),
                settled: Arc::clone(&settled),
                burst,
                shutdown: shutdown.clone(),
                summarizer,
            },
        ));
        Ok(Self {
            settings,
            admission: Mutex::new(Some(admission)),
            counters,
            settled,
            shutdown,
            runner: Mutex::new(Some(runner)),
        })
    }

    #[must_use]
    pub fn enqueue(&self, callback: C) -> Admission {
        let queued = Queued {
            span: Span::current(),
            callback,
        };
        let sent = {
            let admission = self
                .admission
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            admission.as_ref().map(|sender| sender.try_send(queued))
        };
        match sent {
            Some(Ok(())) => {
                self.counters.accepted.fetch_add(1, Ordering::SeqCst);
                Admission::Accepted
            }
            Some(Err(TrySendError::Full(_))) => {
                self.counters.rejected_full.fetch_add(1, Ordering::SeqCst);
                tracing::warn!(
                    capacity = self.settings.max_queue_size,
                    "the logging worker queue is full; the callback was dropped"
                );
                Admission::Rejected(Rejection::QueueFull)
            }
            Some(Err(TrySendError::Closed(_))) | None => {
                self.counters.rejected_closed.fetch_add(1, Ordering::SeqCst);
                tracing::warn!("the logging worker is closed; the callback was dropped");
                Admission::Rejected(Rejection::Closed)
            }
        }
    }

    pub async fn flush(&self) -> FlushReport {
        let target = self.counters.accepted.load(Ordering::SeqCst);
        loop {
            let notified = self.settled.notified();
            pin!(notified);
            notified.as_mut().enable();
            let closed = self
                .admission
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_none();
            if self.counters.settled() >= target || closed {
                break;
            }
            notified.await;
        }
        let stats = self.stats();
        FlushReport {
            accepted: target,
            completed: stats.completed,
            failed: stats.failed,
            timed_out: stats.timed_out,
        }
    }

    pub async fn stop(&self) -> ShutdownReport {
        let admission = self
            .admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        drop(admission);
        let runner = self
            .runner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(mut running) = runner else {
            let stats = self.stats();
            return shutdown_report(stats, stats.pending == 0);
        };
        let report = match timeout(self.settings.shutdown_drain, &mut running).await {
            Ok(Ok(())) => shutdown_report(self.stats(), true),
            Ok(Err(error)) => {
                tracing::error!(%error, "the logging worker task ended unexpectedly");
                shutdown_report(self.stats(), false)
            }
            Err(_) => {
                self.shutdown.cancel();
                let outcome = running.await;
                if let Err(error) = outcome {
                    tracing::error!(%error, "the logging worker task ended unexpectedly");
                }
                shutdown_report(self.stats(), false)
            }
        };
        self.settled.notify_waiters();
        report
    }

    #[must_use]
    pub fn stats(&self) -> WorkerStats {
        let accepted = self.counters.accepted.load(Ordering::SeqCst);
        let completed = self.counters.completed.load(Ordering::SeqCst);
        let failed = self.counters.failed.load(Ordering::SeqCst);
        let timed_out = self.counters.timed_out.load(Ordering::SeqCst);
        let cancelled = self.counters.cancelled.load(Ordering::SeqCst);
        let settled = completed
            .saturating_add(failed)
            .saturating_add(timed_out)
            .saturating_add(cancelled);
        WorkerStats {
            accepted,
            rejected_full: self.counters.rejected_full.load(Ordering::SeqCst),
            rejected_closed: self.counters.rejected_closed.load(Ordering::SeqCst),
            dequeued: self.counters.dequeued.load(Ordering::SeqCst),
            started: self.counters.started.load(Ordering::SeqCst),
            completed,
            failed,
            timed_out,
            cancelled,
            pending: accepted.saturating_sub(settled),
        }
    }
}
