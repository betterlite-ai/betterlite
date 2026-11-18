use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use serde_json::Number;

use crate::budget_reset_policy::phase_failure::{
    EventMetadata, PhaseTiming, RESET_BUDGET_JOB_SERVICE,
};

use super::phase::IdentityPhase;

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseSuccessEvent {
    pub service: &'static str,
    pub call_type: String,
    pub duration_seconds: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub event_metadata: EventMetadata,
}

// service_logging_obj.async_service_success_hook, behind a port so B012's sink and logging stay outside this crate.
pub trait PhaseSuccessSink {
    fn emit(&self, event: PhaseSuccessEvent);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoPhaseSuccessSink;

impl PhaseSuccessSink for NoPhaseSuccessSink {
    fn emit(&self, _event: PhaseSuccessEvent) {}
}

pub fn report_phase_success<Sink>(
    sink: &Sink,
    call_type: &str,
    timing: PhaseTiming,
    event_metadata: EventMetadata,
) where
    Sink: PhaseSuccessSink + ?Sized,
{
    tracing::debug!(call_type = %call_type, "Reset budget phase finished");
    sink.emit(PhaseSuccessEvent {
        service: RESET_BUDGET_JOB_SERVICE,
        call_type: call_type.to_owned(),
        duration_seconds: timing.duration_seconds(),
        start_time: timing.start_time,
        end_time: timing.end_time,
        event_metadata,
    });
}

// A failed chunk reports the rows it found; the success hook adds how many of them reset and how many did not.
#[must_use]
pub(crate) fn found_metadata(phase: IdentityPhase, found: usize) -> EventMetadata {
    let mut metadata = EventMetadata::new();
    metadata.insert(phase.found_count_key(), Json::Number(Number::from(found)));
    metadata
}

#[must_use]
pub(crate) fn outcome_metadata(
    phase: IdentityPhase,
    found: usize,
    updated: usize,
    failed: usize,
) -> EventMetadata {
    let mut metadata = EventMetadata::new();
    metadata.insert(phase.found_count_key(), Json::Number(Number::from(found)));
    metadata.insert(
        phase.updated_count_key(),
        Json::Number(Number::from(updated)),
    );
    metadata.insert(phase.failed_count_key(), Json::Number(Number::from(failed)));
    metadata
}

mod tests;
