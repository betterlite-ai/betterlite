use std::sync::{Mutex, PoisonError};

use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::{debug, info};

use crate::otel_provider_assembly::error::ProviderLookupError;

pub trait ProviderSlot {
    type Provider: Clone;

    fn name(&self) -> &'static str;

    fn existing(&self) -> Result<Option<Self::Provider>, ProviderLookupError>;

    fn publish(&self, provider: &Self::Provider);
}

// Rust cannot downcast the process-wide OpenTelemetry provider, so the "global" provider is this injected slot.
pub struct GlobalSlot<P> {
    pub name: &'static str,
    pub current: Mutex<Option<P>>,
}

impl<P: Clone> ProviderSlot for GlobalSlot<P> {
    type Provider = P;

    fn name(&self) -> &'static str {
        self.name
    }

    fn existing(&self) -> Result<Option<P>, ProviderLookupError> {
        let current = self
            .current
            .lock()
            .map_err(|_poisoned| ProviderLookupError {
                provider: self.name,
            })?;
        Ok(current.clone())
    }

    fn publish(&self, provider: &P) {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        *current = Some(provider.clone());
    }
}

pub struct GlobalProviders {
    pub tracer: GlobalSlot<SdkTracerProvider>,
    pub meter: GlobalSlot<SdkMeterProvider>,
    pub logger: GlobalSlot<SdkLoggerProvider>,
}

impl Default for GlobalProviders {
    fn default() -> Self {
        Self {
            tracer: GlobalSlot {
                name: "TracerProvider",
                current: Mutex::new(None),
            },
            meter: GlobalSlot {
                name: "MeterProvider",
                current: Mutex::new(None),
            },
            logger: GlobalSlot {
                name: "LoggerProvider",
                current: Mutex::new(None),
            },
        }
    }
}

fn create_and_publish<S, E, F>(slot: &S, skip_set_global: bool, create: F) -> Result<S::Provider, E>
where
    S: ProviderSlot,
    F: FnOnce() -> Result<S::Provider, E>,
{
    let provider = create()?;
    if !skip_set_global {
        slot.publish(&provider);
    }
    Ok(provider)
}

fn create_missing<S, E, F>(slot: &S, skip_set_global: bool, create: F) -> Result<S::Provider, E>
where
    S: ProviderSlot,
    F: FnOnce() -> Result<S::Provider, E>,
{
    let name = slot.name();
    debug!(provider = name, "OpenTelemetry: creating a new provider");
    if skip_set_global {
        info!(
            provider = name,
            "OpenTelemetry: creating a provider but NOT setting it globally (will use dynamic providers per-request)"
        );
    }
    create_and_publish(slot, skip_set_global, create)
}

// A failed lookup falls back to a new provider on purpose, so a broken global state never stops startup.
pub fn get_or_create_provider<S, E, F>(
    slot: &S,
    provided: Option<S::Provider>,
    skip_set_global: bool,
    create: F,
) -> Result<S::Provider, E>
where
    S: ProviderSlot,
    F: FnOnce() -> Result<S::Provider, E>,
{
    let name = slot.name();
    if let Some(provider) = provided {
        debug!(
            provider = name,
            "OpenTelemetry: using the provided provider"
        );
        return Ok(provider);
    }
    match slot.existing() {
        Ok(Some(_)) if skip_set_global => {
            debug!(
                provider = name,
                "OpenTelemetry: existing provider found but skip_set_global is set; creating a private one for isolation"
            );
            create()
        }
        Ok(Some(existing)) => {
            debug!(
                provider = name,
                "OpenTelemetry: using the existing provider"
            );
            Ok(existing)
        }
        Ok(None) => create_missing(slot, skip_set_global, create),
        Err(error) => {
            debug!(
                provider = name,
                error = %error,
                "OpenTelemetry: exception checking the existing provider, creating a new one"
            );
            create_and_publish(slot, skip_set_global, create)
        }
    }
}

#[path = "opentelemetry_legacy_provider/tests.rs"]
mod tests;
