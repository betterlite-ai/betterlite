use std::sync::Arc;

use chrono::{DateTime, FixedOffset};
use thiserror::Error;

use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::logging_service_events::sink::ServiceEventSink;
use crate::logging_service_events::types::{
    CANCELLED_ERROR, ServiceEvent, ServiceKind, ServiceLoggerPayload, ServiceType, TerminalOutcome,
};
use crate::logging_service_events::worker_link::Delivery;
use crate::logging_state::correlation::sanitize_correlation_id;
use crate::logging_success::ports::SuccessClock;
use crate::registry::CallbackRegistry;
use crate::worker::{Admission, LoggingWorker, ShutdownReport, WorkerError, WorkerSettings};

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error(transparent)]
    Worker(#[from] WorkerError),
    #[error("the service event redaction patterns could not be built: {source}")]
    Redaction {
        #[source]
        source: RedactionError,
    },
}

pub(crate) struct PipelineConfig {
    pub(crate) settings: WorkerSettings,
    pub(crate) clock: Arc<dyn SuccessClock>,
    pub(crate) sink: Arc<dyn ServiceEventSink>,
    pub(crate) registry: Arc<CallbackRegistry>,
}

// The service event track: every call enqueues its pre-call and terminal events here for B191 to deliver.
pub struct ServicePipeline {
    pub(crate) worker: LoggingWorker<Delivery>,
    clock: Arc<dyn SuccessClock>,
    sink: Arc<dyn ServiceEventSink>,
    registry: Arc<CallbackRegistry>,
    redactor: Redactor,
}

impl ServicePipeline {
    pub(crate) fn start(config: PipelineConfig) -> Result<Self, PipelineError> {
        let worker = LoggingWorker::start(config.settings)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| PipelineError::Redaction { source })?;
        Ok(Self {
            worker,
            clock: config.clock,
            sink: config.sink,
            registry: config.registry,
            redactor,
        })
    }

    #[must_use]
    pub fn begin(self: &Arc<Self>, kind: ServiceKind, call_id: &str) -> ServiceCall {
        let sanitized = sanitize_correlation_id(call_id, Some(&self.redactor));
        let started = self.clock.now();
        let event = ServiceEvent {
            call_id: sanitized.clone(),
            payload: ServiceLoggerPayload {
                is_error: false,
                error: None,
                service: ServiceType::ProxyPreCall,
                duration: 0.0_f64,
                call_type: kind.call_type().to_owned(),
                caller: None,
                event_metadata: None,
            },
            start_time: started,
            end_time: started,
        };
        self.record(event);
        ServiceCall {
            pipeline: Arc::clone(self),
            call_id: sanitized,
            kind,
            started,
            settled: false,
        }
    }

    pub async fn stop(&self) -> ShutdownReport {
        let report = self.worker.stop().await;
        tracing::info!(
            accepted = report.accepted,
            completed = report.completed,
            failed = report.failed,
            timed_out = report.timed_out,
            cancelled = report.cancelled,
            dropped = report.dropped,
            drained = report.drained,
            "logging worker stopped"
        );
        report
    }

    fn record(&self, event: ServiceEvent) {
        let delivery = Delivery {
            sink: Arc::clone(&self.sink),
            registry: Arc::clone(&self.registry),
            event,
        };
        let admission = self.worker.enqueue(delivery);
        if let Admission::Rejected(rejection) = admission {
            tracing::debug!(?rejection, "the service event was not queued");
        }
    }
}

pub struct ServiceCall {
    pipeline: Arc<ServicePipeline>,
    call_id: String,
    kind: ServiceKind,
    started: DateTime<FixedOffset>,
    settled: bool,
}

impl ServiceCall {
    pub(crate) fn finish(mut self, outcome: TerminalOutcome) {
        self.settled = true;
        self.record(outcome);
    }

    fn record(&self, outcome: TerminalOutcome) {
        let end_time = self.pipeline.clock.now();
        let (is_error, error) = match outcome {
            TerminalOutcome::Succeeded => (false, None),
            TerminalOutcome::Failed(message) => (true, Some(message)),
            TerminalOutcome::Cancelled => (true, Some(CANCELLED_ERROR.to_owned())),
        };
        let event = ServiceEvent {
            call_id: self.call_id.clone(),
            payload: ServiceLoggerPayload {
                is_error,
                error,
                service: ServiceType::Litellm,
                duration: seconds_between(self.started, end_time),
                call_type: self.kind.call_type().to_owned(),
                caller: None,
                event_metadata: None,
            },
            start_time: self.started,
            end_time,
        };
        self.pipeline.record(event);
    }
}

impl Drop for ServiceCall {
    fn drop(&mut self) {
        if !self.settled {
            self.settled = true;
            self.record(TerminalOutcome::Cancelled);
        }
    }
}

// A clock that reads backwards records no elapsed time, the way betterlite's other clocks treat it.
fn seconds_between(start: DateTime<FixedOffset>, end: DateTime<FixedOffset>) -> f64 {
    end.signed_duration_since(start)
        .to_std()
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}
