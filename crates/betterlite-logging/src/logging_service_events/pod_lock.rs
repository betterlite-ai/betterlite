use betterlite_db::queues::pod_lock::{PodLockEvent, PodLockEventSink};

use crate::logging_service_events::pipeline::ServicePipeline;
use crate::logging_service_events::types::{
    ServiceEvent, ServiceEventMetadata, ServiceLoggerPayload, ServiceType,
};

impl PodLockEventSink for ServicePipeline {
    fn emit(&self, event: PodLockEvent) {
        let now = self.clock.now();
        self.record(ServiceEvent {
            call_id: String::new(),
            payload: ServiceLoggerPayload {
                is_error: false,
                error: None,
                service: ServiceType::PodLockManager,
                duration: event.duration_seconds,
                call_type: event.call_type.as_str().to_owned(),
                caller: None,
                event_metadata: Some(ServiceEventMetadata {
                    gauge_labels: Some(event.gauge_labels),
                    gauge_value: Some(event.gauge_value),
                }),
            },
            start_time: now,
            end_time: now,
        });
    }
}
