use std::sync::Arc;

use crate::logging_service_events::sink::{ServiceEventError, ServiceEventSink};
use crate::logging_service_events::types::ServiceEvent;
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackPhase;
use crate::registry::sink::CallbackEvent;
use crate::worker::Callback;

// One accepted service event on B191: the registered service callbacks hear the phase and the sink
// writes the event. Nothing here enqueues another event, so a failing callback cannot recurse.
pub struct Delivery {
    pub sink: Arc<dyn ServiceEventSink>,
    pub registry: Arc<CallbackRegistry>,
    pub event: ServiceEvent,
}

impl Callback for Delivery {
    type Error = ServiceEventError;

    fn name(&self) -> &str {
        "service_event"
    }

    async fn call(self) -> Result<(), Self::Error> {
        self.registry.deliver(&CallbackEvent {
            phase: CallbackPhase::Service,
        });
        self.sink.emit(&self.event)
    }
}
