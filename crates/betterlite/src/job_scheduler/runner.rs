use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Notify, watch};
use tokio::task::{Id, JoinError, JoinSet};
use tokio::time::timeout;

use crate::scheduled_job_stagger::{NextRunTime, SubmissionListener, Trigger};

use super::dispatch::{SubmissionEvent, due_run_times, execute_run};
use super::env::JobDefaults;
use super::{Clock, InFlightJob, JobFn, SchedulerState};

pub(crate) struct JobRecord {
    pub(crate) trigger: Arc<dyn Trigger>,
    pub(crate) run: JobFn,
    pub(crate) origin: NextRunTime,
    pub(crate) next_fire: Option<DateTime<Utc>>,
    pub(crate) period_seconds: Option<u32>,
    pub(crate) misfire_grace_seconds: Option<u64>,
    pub(crate) max_instances: Option<u32>,
}

pub(crate) struct InFlightRecord {
    pub(crate) job_id: String,
    pub(crate) finished: watch::Receiver<()>,
}

pub(crate) struct Inner {
    pub(crate) state: SchedulerState,
    pub(crate) jobs: BTreeMap<String, JobRecord>,
    pub(crate) tasks: JoinSet<()>,
    pub(crate) task_runs: BTreeMap<Id, u64>,
    pub(crate) runs: BTreeMap<u64, InFlightRecord>,
    pub(crate) next_run_id: u64,
    pub(crate) listeners: Vec<SubmissionListener>,
}

struct DueJob {
    trigger: Arc<dyn Trigger>,
    run: JobFn,
    next_fire: DateTime<Utc>,
    grace_seconds: u64,
    max_instances: u32,
}

const MAX_SLEEP: Duration = Duration::from_secs(3600);
const MIN_SLEEP: Duration = Duration::from_millis(1);

impl Inner {
    pub(crate) fn reap(&mut self) {
        while let Some(result) = self.tasks.try_join_next_with_id() {
            let (task_id, failure) = match result {
                Ok((task_id, ())) => (task_id, None),
                Err(error) => (error.id(), Some(error)),
            };
            let Some(run_id) = self.task_runs.remove(&task_id) else {
                continue;
            };
            let Some(record) = self.runs.remove(&run_id) else {
                continue;
            };
            if failure.as_ref().is_some_and(JoinError::is_panic) {
                tracing::error!(job_id = %record.job_id, "scheduled job panicked");
            } else {
                tracing::debug!(job_id = %record.job_id, "scheduled job finished");
            }
        }
    }

    pub(crate) fn dispatch_due(&mut self, clock: &Arc<dyn Clock>, defaults: JobDefaults) {
        let now = clock.now();
        let due: Vec<String> = self
            .jobs
            .iter()
            .filter(|(_, job)| job.next_fire.is_some_and(|next| next <= now))
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.dispatch_job(&id, now, clock, defaults);
        }
    }

    pub(crate) fn in_flight_snapshot(&mut self) -> Vec<InFlightJob> {
        self.reap();
        self.runs
            .values()
            .map(|record| InFlightJob {
                job_id: record.job_id.clone(),
                finished: record.finished.clone(),
            })
            .collect()
    }

    fn dispatch_job(
        &mut self,
        id: &str,
        now: DateTime<Utc>,
        clock: &Arc<dyn Clock>,
        defaults: JobDefaults,
    ) {
        let Some(job) = self.due_job(id, defaults) else {
            return;
        };
        let times = due_run_times(job.trigger.as_ref(), job.next_fire, now);
        let Some(last_due) = times.last().copied() else {
            return;
        };
        let advance = job.trigger.next_fire_time(Some(last_due), now);
        if self.instance_limit_reached(id, job.max_instances) {
            tracing::warn!(
                "Execution of job {id} skipped: maximum number of running instances reached ({})",
                job.max_instances
            );
            self.advance(id, advance);
            return;
        }
        let run_times = if defaults.coalesce {
            vec![last_due]
        } else {
            times
        };
        self.submit(id, job, run_times, clock);
        self.advance(id, advance);
    }

    fn due_job(&self, id: &str, defaults: JobDefaults) -> Option<DueJob> {
        let job = self.jobs.get(id)?;
        Some(DueJob {
            trigger: Arc::clone(&job.trigger),
            run: Arc::clone(&job.run),
            next_fire: job.next_fire?,
            grace_seconds: job
                .misfire_grace_seconds
                .unwrap_or(defaults.misfire_grace_seconds),
            max_instances: job.max_instances.unwrap_or(defaults.max_instances),
        })
    }

    fn instance_limit_reached(&self, id: &str, max_instances: u32) -> bool {
        let limit = usize::try_from(max_instances).unwrap_or(usize::MAX);
        let running = self
            .runs
            .values()
            .filter(|record| record.job_id == id)
            .count();
        running >= limit
    }

    fn submit(
        &mut self,
        id: &str,
        job: DueJob,
        run_times: Vec<DateTime<Utc>>,
        clock: &Arc<dyn Clock>,
    ) {
        let event = SubmissionEvent {
            job_id: id.to_owned(),
            run_times: run_times.clone(),
        };
        for listener in &self.listeners {
            listener(&event);
        }
        let job_id = id.to_owned();
        let task_clock = Arc::clone(clock);
        let run_id = self.next_run_id;
        self.next_run_id = self.next_run_id.wrapping_add(1);
        let (finished_tx, finished_rx) = watch::channel(());
        let task = self.tasks.spawn(async move {
            execute_run(job.run, job_id, run_times, job.grace_seconds, task_clock).await;
            let _sent = finished_tx.send(());
        });
        self.task_runs.insert(task.id(), run_id);
        self.runs.insert(
            run_id,
            InFlightRecord {
                job_id: id.to_owned(),
                finished: finished_rx,
            },
        );
    }

    fn advance(&mut self, id: &str, next_fire: Option<DateTime<Utc>>) {
        if let Some(next) = next_fire {
            if let Some(job) = self.jobs.get_mut(id) {
                job.next_fire = Some(next);
            }
            return;
        }
        self.jobs.remove(id);
    }

    fn next_wait(&self, clock: &dyn Clock) -> Option<Duration> {
        let now = clock.now();
        let earliest = self.jobs.values().filter_map(|job| job.next_fire).min()?;
        let delay = earliest.signed_duration_since(now);
        if delay <= TimeDelta::zero() {
            return Some(MIN_SLEEP);
        }
        Some(delay.to_std().unwrap_or(MAX_SLEEP).min(MAX_SLEEP))
    }
}

pub(crate) async fn run_loop(
    inner: Arc<Mutex<Inner>>,
    clock: Arc<dyn Clock>,
    defaults: JobDefaults,
    wake: Arc<Notify>,
) {
    loop {
        let wait = {
            let mut guard = inner.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.state == SchedulerState::Stopped {
                return;
            }
            guard.reap();
            if guard.state == SchedulerState::Running {
                guard.dispatch_due(&clock, defaults);
            }
            if guard.state == SchedulerState::Running {
                guard.next_wait(clock.as_ref())
            } else {
                None
            }
        };
        match wait {
            Some(duration) => {
                let _waited = timeout(duration, wake.notified()).await;
            }
            None => wake.notified().await,
        }
    }
}

mod tests;
