use std::sync::Arc;

use betterlite_redact::secret::Redactor;
use betterlite_router::fallback_observations::observer::FallbackObserver;

mod tests;

// the redactor and the callback list one place hands the router's fallback services
pub struct FallbackObservations {
    pub redactor: Arc<Redactor>,
    pub observers: Vec<Arc<dyn FallbackObserver>>,
}

impl FallbackObservations {
    // get_custom_loggers_for_type: an instance registered twice across the proxy's callback lists fires once
    pub fn register_observer(&mut self, observer: Arc<dyn FallbackObserver>) {
        let registered = self
            .observers
            .iter()
            .any(|existing| Arc::ptr_eq(existing, &observer));
        if !registered {
            self.observers.push(observer);
        }
    }
}
