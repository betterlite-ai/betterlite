use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pyo3::Python;
use tokio::sync::{AcquireError, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, timeout};
use tracing::warn;

use crate::host::{DEFAULT_SHUTDOWN_GRACE, PythonHost};

const DEFAULT_MAX_THREADS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub running: usize,
    pub abandoned: usize,
    pub completed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error("the Python worker pool no longer accepts work")]
    ShuttingDown,
    #[error("the Python worker pool was closed: {source}")]
    Closed {
        #[source]
        source: AcquireError,
    },
    #[error("the Python job did not finish within {after:?}")]
    TimedOut { after: Duration },
    #[error("the Python job panicked: {0}")]
    Panicked(String),
    #[error("the Python job was cancelled before it finished")]
    Cancelled,
}

pub struct WorkTracker {
    shutdown_grace: Duration,
    state: Arc<WorkState>,
}

impl Default for WorkTracker {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_THREADS)
    }
}

impl WorkTracker {
    #[must_use]
    pub fn new(max_threads: usize) -> Self {
        let permits = Arc::new(Semaphore::new(max_threads.min(Semaphore::MAX_PERMITS)));
        Self {
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            state: Arc::new(WorkState {
                permits,
                counters: Mutex::new(Counters::default()),
                idle: Notify::new(),
            }),
        }
    }

    pub async fn run<T, F>(
        &self,
        host: &PythonHost,
        deadline: Duration,
        task: F,
    ) -> Result<T, WorkError>
    where
        T: Send + 'static,
        F: for<'py> FnOnce(Python<'py>) -> T + Send + 'static,
    {
        let job = Arc::new(self.accept()?);
        if let Ok(result) = timeout(deadline, self.execute(host, &job, task)).await {
            drop(job);
            return result;
        }
        job.abandon();
        drop(job);
        Err(WorkError::TimedOut { after: deadline })
    }

    #[must_use]
    pub fn stats(&self) -> Stats {
        let counters = self.state.lock_counters();
        Stats {
            running: counters.running,
            abandoned: counters.abandoned,
            completed: counters.completed,
        }
    }

    pub async fn shutdown(&self) {
        self.stop_accepting();
        let started = Instant::now();
        loop {
            let pending = self.state.lock_counters().active;
            if pending == 0 {
                return;
            }
            let remaining = self.shutdown_grace.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                warn!(
                    jobs = pending,
                    "Python worker pool shutdown grace elapsed with jobs still running"
                );
                return;
            }
            let idle = self.state.idle.notified();
            tokio::pin!(idle);
            idle.as_mut().enable();
            if self.state.lock_counters().active == 0 {
                return;
            }
            if timeout(remaining, idle.as_mut()).await.is_ok() {
                continue;
            }
            let remaining_jobs = self.state.lock_counters().active;
            if remaining_jobs > 0 {
                warn!(
                    jobs = remaining_jobs,
                    "Python worker pool shutdown grace elapsed with jobs still running"
                );
            }
            return;
        }
    }

    fn stop_accepting(&self) {
        let mut counters = self.state.lock_counters();
        counters.accepting = false;
        drop(counters);
    }

    fn accept(&self) -> Result<TrackedJob, WorkError> {
        let mut counters = self.state.lock_counters();
        if !counters.accepting {
            return Err(WorkError::ShuttingDown);
        }
        counters.active = counters.active.saturating_add(1);
        drop(counters);
        Ok(TrackedJob {
            state: Arc::clone(&self.state),
            progress: Mutex::new(JobProgress::default()),
            permit: Mutex::new(None),
        })
    }

    async fn execute<T, F>(
        &self,
        host: &PythonHost,
        job: &Arc<TrackedJob>,
        task: F,
    ) -> Result<T, WorkError>
    where
        T: Send + 'static,
        F: for<'py> FnOnce(Python<'py>) -> T + Send + 'static,
    {
        job.begin(
            Arc::clone(&self.state.permits)
                .acquire_owned()
                .await
                .map_err(|source| WorkError::Closed { source })?,
        );
        let blocking_host = host.clone();
        let blocking_job = Arc::clone(job);
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            // The job owns the pool permit, so it has to outlive the Python call.
            let _job = blocking_job;
            blocking_host.attach(task)
        });
        match tasks.join_next().await {
            Some(Ok(value)) => Ok(value),
            Some(Err(error)) => Err(join_failure(error)),
            None => Err(WorkError::Cancelled),
        }
    }
}

struct WorkState {
    permits: Arc<Semaphore>,
    counters: Mutex<Counters>,
    idle: Notify,
}

impl WorkState {
    fn lock_counters(&self) -> MutexGuard<'_, Counters> {
        match self.counters.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

struct Counters {
    running: usize,
    abandoned: usize,
    completed: usize,
    active: usize,
    // Guarded by the counters lock so shutdown cannot refuse an already accepted job.
    accepting: bool,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            running: 0,
            abandoned: 0,
            completed: 0,
            active: 0,
            accepting: true,
        }
    }
}

struct TrackedJob {
    state: Arc<WorkState>,
    progress: Mutex<JobProgress>,
    permit: Mutex<Option<OwnedSemaphorePermit>>,
}

#[derive(Default)]
struct JobProgress {
    phase: Phase,
    abandoned: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    #[default]
    Waiting,
    Running,
}

impl TrackedJob {
    fn lock_progress(&self) -> MutexGuard<'_, JobProgress> {
        match self.progress.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn begin(&self, permit: OwnedSemaphorePermit) {
        *self.permit.lock().unwrap_or_else(PoisonError::into_inner) = Some(permit);
        let mut progress = self.lock_progress();
        progress.phase = Phase::Running;
        let mut counters = self.state.lock_counters();
        counters.running = counters.running.saturating_add(1);
        drop(counters);
        drop(progress);
    }

    fn abandon(&self) {
        let mut progress = self.lock_progress();
        if progress.phase != Phase::Running || progress.abandoned {
            return;
        }
        progress.abandoned = true;
        let mut counters = self.state.lock_counters();
        counters.abandoned = counters.abandoned.saturating_add(1);
        drop(counters);
        drop(progress);
    }
}

impl Drop for TrackedJob {
    fn drop(&mut self) {
        let progress = self.lock_progress();
        let mut counters = self.state.lock_counters();
        counters.active = counters.active.saturating_sub(1);
        let running = progress.phase == Phase::Running;
        let abandoned = progress.abandoned;
        if running {
            counters.running = counters.running.saturating_sub(1);
            counters.completed = counters.completed.saturating_add(1);
            counters.abandoned = counters.abandoned.saturating_sub(usize::from(abandoned));
        }
        drop(counters);
        drop(progress);
        self.state.idle.notify_waiters();
    }
}

fn join_failure(error: JoinError) -> WorkError {
    if !error.is_panic() {
        return WorkError::Cancelled;
    }
    let payload = error.into_panic();
    if let Some(message) = payload.downcast_ref::<&str>() {
        return WorkError::Panicked((*message).to_owned());
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return WorkError::Panicked(message.clone());
    }
    WorkError::Panicked("the job panicked without a message".to_owned())
}

mod tests;
