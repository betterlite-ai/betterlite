use std::time::Duration;

use tokio::time::{Instant, timeout};

use crate::scheduled_job_stagger::JobScheduler as _;

use super::{InFlightJob, ProxyScheduler};

pub fn pause_scheduled_jobs(scheduler: &mut ProxyScheduler) {
    if scheduler.running() {
        scheduler.pause();
    }
}

pub async fn stop_in_flight_jobs(
    scheduler: &mut ProxyScheduler,
    finish: Duration,
    cancel: Duration,
) {
    if !scheduler.running() {
        return;
    }
    let in_flight = scheduler.in_flight();
    if !in_flight.is_empty() {
        tracing::info!(
            "Waiting up to {}s for {} in-flight scheduled job(s) to finish",
            finish.as_secs_f64(),
            in_flight.len()
        );
    }
    let still_running = wait_finished(in_flight, finish).await;
    scheduler.shutdown();
    if still_running.is_empty() {
        return;
    }
    tracing::info!(
        "Cancelling {} in-flight scheduled job(s) for shutdown",
        still_running.len()
    );
    let pending = wait_finished(still_running, cancel).await;
    if !pending.is_empty() {
        tracing::warn!(
            "{} scheduled job(s) did not finish within {}s of cancellation; giving up on them",
            pending.len(),
            cancel.as_secs_f64()
        );
    }
}

async fn wait_finished(jobs: Vec<InFlightJob>, limit: Duration) -> Vec<InFlightJob> {
    let deadline = Instant::now().checked_add(limit);
    let mut pending = Vec::new();
    for mut job in jobs {
        let remaining = deadline.map_or(limit, |end| end.saturating_duration_since(Instant::now()));
        if timeout(remaining, job.finished.changed()).await.is_err() {
            pending.push(job);
        }
    }
    pending
}

mod tests;
