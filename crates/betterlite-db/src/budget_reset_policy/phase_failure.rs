use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;

// ServiceTypes.RESET_BUDGET_JOB: B012's service type, carried as the value the service-logging hook records.
pub const RESET_BUDGET_JOB_SERVICE: &str = "reset_budget_job";

// The hook takes LiteLLM's dict[str, object], so a value is the repo's typed JSON tree; a two-variant count-or-flag enum would also trip variant_size_differences.
pub type EventMetadata = BTreeMap<&'static str, Json>;

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseFailureEvent {
    pub service: &'static str,
    pub call_type: String,
    pub error: String,
    pub duration_seconds: f64,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub event_metadata: EventMetadata,
}

pub trait PhaseFailureSink {
    fn emit(&self, event: PhaseFailureEvent);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoPhaseFailureSink;

impl PhaseFailureSink for NoPhaseFailureSink {
    fn emit(&self, _event: PhaseFailureEvent) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseTiming {
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

impl PhaseTiming {
    // A clock that reads backwards records no elapsed time, the way betterlite's logging pipeline treats it.
    #[must_use]
    pub fn duration_seconds(self) -> f64 {
        self.end_time
            .signed_duration_since(self.start_time)
            .to_std()
            .map_or(0.0, |elapsed| elapsed.as_secs_f64())
    }
}

// _emit_phase_failure: rows that did not reset are reported without failing the chunk, carrying the timing and metadata the service failure hook records.
pub fn report_phase_failure<Sink>(
    sink: &Sink,
    call_type: &str,
    error: &str,
    timing: PhaseTiming,
    event_metadata: EventMetadata,
) where
    Sink: PhaseFailureSink + ?Sized,
{
    tracing::error!(
        call_type = %call_type,
        failure = %error,
        "Reset budget phase failed"
    );
    sink.emit(PhaseFailureEvent {
        service: RESET_BUDGET_JOB_SERVICE,
        call_type: call_type.to_owned(),
        error: error.to_owned(),
        duration_seconds: timing.duration_seconds(),
        start_time: timing.start_time,
        end_time: timing.end_time,
        event_metadata,
    });
}

mod tests;
