use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use croner::errors::CronError;
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;

use crate::scheduled_job_stagger::{NextRunTime, Trigger};

mod dispatch;
pub mod env;
mod fields;
pub mod jobs;
pub mod runner;
pub mod shutdown;
pub mod triggers;

mod tests;

pub type BoxFuture<'future, T> = Pin<Box<dyn Future<Output = T> + Send + 'future>>;

pub type JobFn = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SchedulerState {
    #[default]
    Stopped,
    Running,
    Paused,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("Wrong number of fields; got {got}, expected 5")]
    Crontab { got: usize },
    #[error(
        "Error validating expression '{expression}': the step value ({step}) is higher than the total range of the expression ({range})"
    )]
    CronStep {
        expression: String,
        step: u64,
        range: u64,
    },
    #[error(
        "Error validating expression '{expression}': the first value ({value}) is lower than the minimum value ({minimum})"
    )]
    CronBelowMin {
        expression: String,
        value: u64,
        minimum: u64,
    },
    #[error(
        "Error validating expression '{expression}': the last value ({value}) is higher than the maximum value ({maximum})"
    )]
    CronAboveMax {
        expression: String,
        value: u64,
        maximum: u64,
    },
    #[error("The minimum value in a range must not be higher than the maximum")]
    CronRangeOrder,
    #[error("Increment must be higher than 0")]
    CronStepZero,
    #[error("Unrecognized expression \"{expression}\" for field \"{field}\"")]
    CronExpression {
        field: &'static str,
        expression: String,
    },
    #[error("Invalid {field} name \"{name}\"")]
    CronName { field: &'static str, name: String },
    #[error(transparent)]
    Cron(#[from] CronError),
    #[error("environment variable {name} has an invalid value")]
    InvalidEnv { name: String },
    #[error("a scheduled job with id {id} already exists")]
    ConflictingId { id: String },
    #[error("no scheduled job with id {id}")]
    JobLookup { id: String },
}

pub struct JobSpec {
    pub id: String,
    pub trigger: Box<dyn Trigger>,
    pub run: JobFn,
    pub replace_existing: bool,
    pub next_run_time: NextRunTime,
    pub misfire_grace_seconds: Option<u64>,
    pub max_instances: Option<u32>,
}

pub struct JobView {
    pub id: String,
    pub next_run_time: NextRunTime,
    pub scheduled_for: Option<DateTime<Utc>>,
    pub period_seconds: Option<u32>,
    pub trigger: Box<dyn Trigger>,
}

pub struct InFlightJob {
    pub job_id: String,
    pub finished: watch::Receiver<()>,
}

pub struct ProxyScheduler {
    defaults: env::JobDefaults,
    clock: Arc<dyn Clock>,
    inner: Arc<Mutex<runner::Inner>>,
    wake: Arc<Notify>,
    loop_tasks: JoinSet<()>,
}

impl ProxyScheduler {
    pub fn new(defaults: env::JobDefaults, clock: Arc<dyn Clock>) -> Self {
        Self {
            defaults,
            clock,
            inner: Arc::new(Mutex::new(runner::Inner {
                state: SchedulerState::Stopped,
                jobs: BTreeMap::new(),
                tasks: JoinSet::new(),
                task_runs: BTreeMap::new(),
                runs: BTreeMap::new(),
                next_run_id: 0,
                listeners: Vec::new(),
            })),
            wake: Arc::new(Notify::new()),
            loop_tasks: JoinSet::new(),
        }
    }

    pub fn add_job(&mut self, spec: JobSpec) -> Result<(), SchedulerError> {
        let now = self.clock.now();
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.jobs.contains_key(&spec.id) && !spec.replace_existing {
            return Err(SchedulerError::ConflictingId { id: spec.id });
        }
        let next_fire = match spec.next_run_time {
            NextRunTime::Unset => spec.trigger.next_fire_time(None, now),
            NextRunTime::Explicit(value) => value,
        };
        let period_seconds = triggers::interval_seconds(spec.trigger.as_ref(), now);
        let trigger: Arc<dyn Trigger> = Arc::from(spec.trigger);
        inner.jobs.insert(
            spec.id,
            runner::JobRecord {
                trigger,
                run: spec.run,
                origin: spec.next_run_time,
                next_fire,
                period_seconds,
                misfire_grace_seconds: spec.misfire_grace_seconds,
                max_instances: spec.max_instances,
            },
        );
        let running = inner.state == SchedulerState::Running;
        drop(inner);
        if running {
            self.wake.notify_one();
        }
        Ok(())
    }

    #[must_use]
    pub fn get_job(&self, id: &str) -> Option<JobView> {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let job = inner.jobs.get(id)?;
        let next_run_time = job.origin;
        let scheduled_for = job.next_fire;
        let period_seconds = job.period_seconds;
        let trigger = Arc::clone(&job.trigger);
        drop(inner);
        Some(JobView {
            id: id.to_owned(),
            next_run_time,
            scheduled_for,
            period_seconds,
            trigger: Box::new(trigger),
        })
    }

    pub fn remove_job(&mut self, id: &str) -> Result<(), SchedulerError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.jobs.remove(id).is_none() {
            return Err(SchedulerError::JobLookup { id: id.to_owned() });
        }
        drop(inner);
        Ok(())
    }

    pub fn start(&mut self) {
        let Ok(handle) = Handle::try_current() else {
            tracing::error!("the job scheduler needs a tokio runtime to start");
            return;
        };
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.state != SchedulerState::Stopped {
            return;
        }
        inner.state = SchedulerState::Running;
        drop(inner);
        self.loop_tasks = JoinSet::new();
        let loop_inner = Arc::clone(&self.inner);
        let loop_clock = Arc::clone(&self.clock);
        let loop_wake = Arc::clone(&self.wake);
        self.loop_tasks.spawn_on(
            runner::run_loop(loop_inner, loop_clock, self.defaults, loop_wake),
            &handle,
        );
        self.wake.notify_one();
    }

    pub fn pause(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.state != SchedulerState::Running {
            return;
        }
        inner.state = SchedulerState::Paused;
        drop(inner);
        self.wake.notify_one();
    }

    pub fn resume(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.state == SchedulerState::Paused {
            inner.state = SchedulerState::Running;
            drop(inner);
            self.wake.notify_one();
        }
    }

    #[must_use]
    pub fn state(&self) -> SchedulerState {
        match self.inner.lock() {
            Ok(inner) => inner.state,
            Err(poisoned) => poisoned.into_inner().state,
        }
    }

    pub fn shutdown(&mut self) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.state == SchedulerState::Stopped {
            return;
        }
        inner.state = SchedulerState::Stopped;
        inner.tasks.abort_all();
        inner.task_runs.clear();
        inner.runs.clear();
        drop(inner);
        self.loop_tasks.abort_all();
        self.wake.notify_one();
    }

    #[must_use]
    pub fn in_flight(&mut self) -> Vec<InFlightJob> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let running = inner.in_flight_snapshot();
        drop(inner);
        running
    }
}

// A job snapshot carries its trigger out, so the scheduler stores triggers behind an Arc to keep its own copy.
impl Trigger for Arc<dyn Trigger> {
    fn next_fire_time(
        &self,
        previous_fire_time: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        self.as_ref().next_fire_time(previous_fire_time, now)
    }
}
