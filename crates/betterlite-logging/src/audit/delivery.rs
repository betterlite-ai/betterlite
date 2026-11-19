use std::sync::Arc;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::worker::{Admission, Callback, LoggingWorker, ShutdownReport};

use super::callbacks::NamedAuditCallbacks;
use super::payload::StandardAuditLogPayload;

pub type AuditCallback = Arc<dyn CallbackLogger>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchReport {
    pub submitted: usize,
    pub rejected: usize,
}

pub struct AuditCallbackJob {
    callback: AuditCallback,
    payload: StandardAuditLogPayload,
}

impl Callback for AuditCallbackJob {
    type Error = LoggerError;

    fn name(&self) -> &str {
        self.callback.callback_name()
    }

    async fn call(self) -> Result<(), LoggerError> {
        self.callback.async_log_audit_log_event(&self.payload).await
    }
}

// The worker must start inside a runtime, and it keeps a slow audit callback from holding the caller.
pub struct AuditDelivery {
    pub named: NamedAuditCallbacks,
    pub worker: LoggingWorker<AuditCallbackJob>,
}

impl AuditDelivery {
    #[must_use]
    pub fn dispatch(
        &self,
        callbacks: &[AuditCallback],
        payload: &StandardAuditLogPayload,
    ) -> DispatchReport {
        let mut report = DispatchReport::default();
        for callback in callbacks {
            let job = AuditCallbackJob {
                callback: Arc::clone(callback),
                payload: payload.clone(),
            };
            match self.worker.enqueue(job) {
                Admission::Accepted => {
                    report.submitted = report.submitted.saturating_add(1);
                }
                Admission::Rejected(rejection) => {
                    report.rejected = report.rejected.saturating_add(1);
                    tracing::error!(
                        callback = %callback.callback_name(),
                        ?rejection,
                        "the audit log callback was rejected; its delivery was dropped"
                    );
                }
            }
        }
        report
    }

    pub async fn shutdown(&self) -> ShutdownReport {
        let report = self.worker.stop().await;
        tracing::info!(
            accepted = report.accepted,
            completed = report.completed,
            failed = report.failed,
            timed_out = report.timed_out,
            cancelled = report.cancelled,
            dropped = report.dropped,
            "the audit callback delivery worker stopped"
        );
        report
    }
}
