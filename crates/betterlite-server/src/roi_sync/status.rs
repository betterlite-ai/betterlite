use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::search_tools::convert::isoformat;
use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus, ROISyncPhase, ROISyncStatus};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

use super::PR_CONCURRENCY;
use super::ports::SyncClock;

pub(crate) const IDLE_STAGE: &str = "Idle";
pub(crate) const SPEND_STAGE: &str = "Reading gateway spend";
pub(crate) const REPOSITORIES_STAGE: &str = "Reading configured repositories";
pub(crate) const ESTIMATES_STAGE: &str = "Estimating new or changed pull requests";
pub(crate) const CANCELLED_STAGE: &str = "Sync cancelled";
pub(crate) const FAILED_STAGE: &str = "Sync failed";

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
    let elapsed = finish
        .unwrap_or(now)
        .signed_duration_since(started)
        .num_seconds();
    elapsed.max(0)
}

fn remaining_seconds(status: &ROISyncStatus, elapsed: i64) -> Option<i64> {
    let enough = usize::try_from(status.done).is_ok_and(|done| done >= PR_CONCURRENCY);
    if !status.running || !enough {
        return None;
    }
    let span = status.total.checked_sub(status.done)?;
    let numerator = Decimal::from(elapsed).checked_mul(Decimal::from(span))?;
    let estimate = numerator.checked_div(Decimal::from(status.done))?;
    let rounded = estimate.round_dp_with_strategy(0, RoundingStrategy::MidpointNearestEven);
    let seconds = i64::try_from(rounded).ok()?;
    Some(seconds.max(0))
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
                REPOSITORIES_STAGE.clone_into(&mut status.stage);
            }
            StatusChange::Estimates { total } => {
                status.phase = ROISyncPhase::Estimates;
                ESTIMATES_STAGE.clone_into(&mut status.stage);
                status.total = total;
            }
            StatusChange::Failed { message } => {
                status.phase = ROISyncPhase::Error;
                FAILED_STAGE.clone_into(&mut status.stage);
                status.error = Some(message);
            }
            StatusChange::Cancelled => {
                status.running = false;
                status.phase = ROISyncPhase::Cancelled;
                CANCELLED_STAGE.clone_into(&mut status.stage);
            }
            StatusChange::Finished => {
                status.running = false;
                status.finished_at = Some(isoformat(now));
            }
            StatusChange::FinishedComputed => {
                status.running = false;
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
        drop(stored);
        completed.running = false;
        completed.phase = ROISyncPhase::Complete;
        stage.clone_into(&mut completed.stage);
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
