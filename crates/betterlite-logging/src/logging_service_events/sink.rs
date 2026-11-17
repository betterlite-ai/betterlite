use thiserror::Error;

use crate::logging_service_events::types::ServiceEvent;
use crate::logging_service_events::types::wire_names;

#[derive(Debug, Error)]
pub enum ServiceEventError {
    #[error("the service event sink rejected the event: {message}")]
    Rejected { message: String },
}

pub trait ServiceEventSink: Send + Sync {
    fn emit(&self, event: &ServiceEvent) -> Result<(), ServiceEventError>;
}

// B003's stderr format carries the event: one JSON record with the identity, the service and its timing
pub struct TracingSink;

impl ServiceEventSink for TracingSink {
    fn emit(&self, event: &ServiceEvent) -> Result<(), ServiceEventError> {
        let payload = &event.payload;
        tracing::info!(
            call_id = %event.call_id,
            service = wire_names::as_str(payload.service),
            call_type = %payload.call_type,
            is_error = payload.is_error,
            duration = payload.duration,
            error = payload.error.as_deref().unwrap_or_default(),
            "service event"
        );
        Ok(())
    }
}
