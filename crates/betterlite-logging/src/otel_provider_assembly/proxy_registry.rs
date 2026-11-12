use std::sync::{Mutex, PoisonError};

const OTEL_SERVICE_CALLBACK: &str = "otel";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtelRegistration {
    pub callback_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceCallback {
    Named(String),
    OpenTelemetry(OtelRegistration),
}

// The proxy's service callback list and `open_telemetry_logger` slot, injected because Rust keeps no module globals.
#[derive(Default)]
pub struct ProxyRegistry {
    pub service_callbacks: Mutex<Vec<ServiceCallback>>,
    pub open_telemetry_logger: Mutex<Option<OtelRegistration>>,
}

// The first logger to register owns the proxy slot, so a later Langfuse or Arize logger cannot replace it.
pub fn register_on_proxy(registry: &ProxyRegistry, logger: &OtelRegistration) {
    let mut callbacks = registry
        .service_callbacks
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let present = callbacks.iter().any(|callback| match callback {
        ServiceCallback::Named(name) => name == OTEL_SERVICE_CALLBACK,
        ServiceCallback::OpenTelemetry(_) => true,
    });
    if !present {
        callbacks.push(ServiceCallback::OpenTelemetry(logger.clone()));
    }
    drop(callbacks);
    let mut slot = registry
        .open_telemetry_logger
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(logger.clone());
    }
    drop(slot);
}

#[path = "proxy_registry/tests.rs"]
mod tests;
