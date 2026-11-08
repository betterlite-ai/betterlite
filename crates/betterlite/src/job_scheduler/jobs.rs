use std::sync::{Arc, PoisonError};

use crate::scheduled_job_stagger::{
    JobScheduler, JobSnapshot, NextRunTime, SubmissionListener, Trigger,
};

use super::triggers;
use super::{ProxyScheduler, SchedulerState};

impl JobScheduler for ProxyScheduler {
    fn running(&self) -> bool {
        self.state() != SchedulerState::Stopped
    }

    fn jobs(&mut self) -> Vec<JobSnapshot> {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let snapshots = inner
            .jobs
            .iter()
            .map(|(id, job)| JobSnapshot {
                id: id.clone(),
                next_run_time: job.origin,
                period_seconds: job.period_seconds,
                trigger: Box::new(Arc::clone(&job.trigger)),
            })
            .collect();
        drop(inner);
        snapshots
    }

    fn modify_job(&mut self, job_id: &str, trigger: Box<dyn Trigger>) {
        let now = self.clock.now();
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(job) = inner.jobs.get_mut(job_id) else {
            tracing::warn!(
                job_id,
                "scheduled job not found; its trigger was not modified"
            );
            return;
        };
        // A job an explicit next_run_time paused stays paused across a trigger swap.
        let next_fire = match job.origin {
            NextRunTime::Explicit(None) => None,
            NextRunTime::Unset | NextRunTime::Explicit(Some(_)) => {
                trigger.next_fire_time(None, now)
            }
        };
        let period_seconds = triggers::interval_seconds(trigger.as_ref(), now);
        let stored_trigger: Arc<dyn Trigger> = Arc::from(trigger);
        job.period_seconds = period_seconds;
        job.next_fire = next_fire;
        job.trigger = stored_trigger;
        let running = inner.state == SchedulerState::Running;
        drop(inner);
        if running {
            self.wake.notify_one();
        }
    }

    fn add_submission_listener(&mut self, listener: SubmissionListener) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.listeners.push(listener);
    }
}
