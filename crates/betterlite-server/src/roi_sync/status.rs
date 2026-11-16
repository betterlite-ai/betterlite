use std::sync::{Arc, Mutex, PoisonError};

use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus, ROISyncPhase, ROISyncStatus};
use chrono::{DateTime, Utc};

use super::PR_CONCURRENCY;
use super::ports::SyncClock;

pub(crate) const IDLE_STAGE: &str = "Idle";
pub(crate) const SPEND_STAGE: &str = "Reading gateway spend";
pub(crate) const REPOSITORIES_STAGE: &str = "Reading configured repositories";
pub(crate) const ESTIMATES_STAGE: &str = "Estimating new or changed pull requests";
pub(crate) const CANCELLED_STAGE: &str = "Sync cancelled";
pub(crate) const FAILED_STAGE: &str = "Sync failed";

// Python's datetime.isoformat: the fraction appears only when the microseconds are not zero
pub(crate) fn isoformat(time: DateTime<Utc>) -> String {
    if time.timestamp_subsec_micros() == 0 {
        time.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
    } else {
        time.format("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
    }
}

pub(crate) fn idle_status() -> ROISyncStatus {
    ROISyncStatus {
        running: false,
        phase: ROISyncPhase::Idle,
        stage: IDLE_STAGE.to_owned(),
        done: 0,
        total: 0,
        estimated: 0,
        reused: 0,
        needs_attention: 0,
        error: None,
        started_at: None,
        finished_at: None,
        next_update: None,
        elapsed_seconds: 0,
        remaining_seconds: None,
    }
}

pub(crate) fn initial_status(now: DateTime<Utc>) -> ROISyncStatus {
    ROISyncStatus {
        running: true,
        phase: ROISyncPhase::Spend,
        stage: SPEND_STAGE.to_owned(),
        started_at: Some(isoformat(now)),
        ..idle_status()
    }
}

pub(crate) fn timed_status(status: ROISyncStatus, now: DateTime<Utc>) -> ROISyncStatus {
    if status.started_at.is_none() {
        return status;
    }
    let elapsed = elapsed_seconds(&status, now);
    let remaining = remaining_seconds(&status, elapsed);
    ROISyncStatus {
        elapsed_seconds: elapsed,
        remaining_seconds: remaining,
        ..status
    }
}

fn elapsed_seconds(status: &ROISyncStatus, now: DateTime<Utc>) -> i64 {
    let Some(started_text) = status.started_at.as_deref() else {
        return 0;
    };
    let Ok(started) = DateTime::parse_from_rfc3339(started_text) else {
        return 0;
    };
    let finish = status
        .finished_at
        .as_deref()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok());
    let elapsed = match finish {
        Some(finished) => finished.signed_duration_since(started).num_seconds(),
        None => now.signed_duration_since(started).num_seconds(),
    };
    elapsed.max(0)
}

fn remaining_seconds(status: &ROISyncStatus, elapsed: i64) -> Option<i64> {
    let enough = usize::try_from(status.done).is_ok_and(|done| done >= PR_CONCURRENCY);
    if !status.running || !enough {
        return None;
    }
    let span = status.total.checked_sub(status.done)?;
    let numerator = i128::from(elapsed).checked_mul(i128::from(span))?;
    let rounded = round_half_even(numerator, i128::from(status.done))?;
    Some(rounded.max(0))
}

fn round_half_even(numerator: i128, denominator: i128) -> Option<i64> {
    let quotient = numerator.checked_div(denominator)?;
    let remainder = numerator.checked_rem(denominator)?;
    let doubled = remainder.checked_abs()?.checked_mul(2)?;
    let tie_breaks_up = doubled == denominator && quotient.rem_euclid(2) != 0;
    let rounded = if doubled > denominator || tie_breaks_up {
        quotient.checked_add(numerator.signum())?
    } else {
        quotient
    };
    i64::try_from(rounded).ok()
}

#[derive(Clone)]
pub(crate) struct StatusReporter {
    pub(crate) stored: Arc<Mutex<ROISyncStatus>>,
    pub(crate) clock: SyncClock,
}

impl StatusReporter {
    pub(crate) fn snapshot(&self) -> ROISyncStatus {
        let now = (self.clock)();
        let stored = self
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        timed_status(stored, now)
    }

    pub(crate) fn apply(&self, change: StatusChange) {
        let now = (self.clock)();
        let mut status = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        match change {
            StatusChange::Repositories => {
                status.phase = ROISyncPhase::Repositories;
                status.stage = REPOSITORIES_STAGE.to_owned();
            }
            StatusChange::Estimates { total } => {
                status.phase = ROISyncPhase::Estimates;
                status.stage = ESTIMATES_STAGE.to_owned();
                status.total = total;
            }
            StatusChange::Failed { message } => {
                status.phase = ROISyncPhase::Error;
                status.stage = FAILED_STAGE.to_owned();
                status.error = Some(message);
            }
            StatusChange::Cancelled => {
                status.running = false;
                status.phase = ROISyncPhase::Cancelled;
                status.stage = CANCELLED_STAGE.to_owned();
            }
            StatusChange::Finished => {
                status.running = false;
                status.finished_at = Some(isoformat(now));
            }
            StatusChange::FinishedComputed => {
                let finished = timed_status(status.clone(), now);
                *status = finished;
                status.finished_at = Some(isoformat(now));
            }
        }
    }

    pub(crate) fn completed_status(&self, stage: &str) -> ROISyncStatus {
        let now = (self.clock)();
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        let mut completed = timed_status(stored.clone(), now);
        completed.running = false;
        completed.phase = ROISyncPhase::Complete;
        completed.stage = stage.to_owned();
        completed.finished_at = Some(isoformat(now));
        completed
    }

    pub(crate) fn record_estimate(&self, estimate: &ROIEstimate) {
        let estimated = estimate.status == ROIEstimateStatus::Estimated;
        let reused = estimate.cached == Some(true);
        let mut status = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        status.done = status.done.saturating_add(1);
        if estimated {
            status.estimated = status.estimated.saturating_add(1);
        } else {
            status.needs_attention = status.needs_attention.saturating_add(1);
        }
        if reused {
            status.reused = status.reused.saturating_add(1);
        }
    }
}

pub(crate) enum StatusChange {
    Repositories,
    Estimates { total: i64 },
    Failed { message: String },
    Cancelled,
    Finished,
    FinishedComputed,
}
