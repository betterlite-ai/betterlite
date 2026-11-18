use std::error::Error as StdError;
use std::sync::Arc;

use betterlite_logging::logging_service_events::sink::ServiceEventSink;
use betterlite_logging::logging_service_events::types::{
    ServiceEvent as LoggedServiceEvent, ServiceLoggerPayload, ServiceType,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use chrono::TimeDelta;
use uuid::Uuid;

// Every cache shares this; the sink is sync by contract, so a cache method never spawns a task.
pub struct ServiceEventHooks {
    pub sink: Arc<dyn ServiceEventSink>,
    pub clock: Arc<dyn SuccessClock>,
}

impl ServiceHooks for ServiceEventHooks {
    fn success(&self, event: &ServiceEvent) {
        self.emit(event, None);
    }

    fn failure(&self, event: &ServiceEvent, error: &(dyn StdError + Send + Sync)) {
        self.emit(event, Some(error.to_string()));
    }
}

impl ServiceEventHooks {
    fn emit(&self, event: &ServiceEvent, error: Option<String>) {
        let end_time = self.clock.now();
        let elapsed = TimeDelta::from_std(event.duration).unwrap_or(TimeDelta::MAX);
        let is_error = error.is_some();
        let logged = LoggedServiceEvent {
            call_id: Uuid::new_v4().to_string(),
            payload: ServiceLoggerPayload {
                is_error,
                error,
                service: ServiceType::Redis,
                duration: event.duration.as_secs_f64(),
                call_type: event.call_type.to_owned(),
                caller: None,
                event_metadata: None,
            },
            start_time: end_time - elapsed,
            end_time,
        };
        if let Err(source) = self.sink.emit(&logged) {
            tracing::warn!(error = %source, "the redis service event was not emitted");
        }
    }
}
