use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::InMemorySpanExporter;

use super::super::{CatalogError, CatalogInputs};
use super::{env, otlp_http_clients};
use crate::otel::plumbing::exporters::HttpClients;
use crate::otel_provider_assembly::opentelemetry_legacy_provider::GlobalProviders;
use crate::otel_provider_assembly::proxy_registry::ProxyRegistry;

// What every legacy arm shares: one environment snapshot, one set of provider slots, one exporter pool.
pub struct SharedOtel {
    pub env: BTreeMap<String, String>,
    pub globals: GlobalProviders,
    pub proxy: ProxyRegistry,
    pub shared_exporter: InMemorySpanExporter,
    clients: Mutex<Option<HttpClients>>,
    providers: Mutex<Vec<KeptProviders>>,
}

// LiteLLM keeps the meter and logger providers on the logger; the recorder and emitter hold handles only.
struct KeptProviders {
    _meter: Option<SdkMeterProvider>,
    _logger: Option<SdkLoggerProvider>,
}

impl SharedOtel {
    pub fn new(inputs: &CatalogInputs) -> Self {
        Self {
            env: env::snapshot(inputs),
            globals: GlobalProviders::default(),
            proxy: ProxyRegistry::default(),
            shared_exporter: InMemorySpanExporter::default(),
            clients: Mutex::new(None),
            providers: Mutex::new(Vec::new()),
        }
    }

    // Three OtlpClients are three threads, so an arm that never exports pays for none of them.
    pub fn clients(&self, inputs: &CatalogInputs) -> Result<HttpClients, CatalogError> {
        let mut slot = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(clients) = slot.as_ref() {
            return Ok(clients.clone());
        }
        let clients = otlp_http_clients(inputs)?;
        *slot = Some(clients.clone());
        Ok(clients)
    }

    pub fn keep(&self, meter: Option<SdkMeterProvider>, logger: Option<SdkLoggerProvider>) {
        if meter.is_none() && logger.is_none() {
            return;
        }
        let kept = KeptProviders {
            _meter: meter,
            _logger: logger,
        };
        self.providers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kept);
    }
}
