use std::sync::{Arc, Mutex, MutexGuard};

use betterlite_config::loader::EnvironmentLookup;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::otel::config::Settings;
use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::fan_out::attach::TenantFanOutRegistry;
use crate::otel::fan_out::scope::RequestScopeSource;
use crate::otel::logger::v2::RequestTraceSource;
use crate::otel::plumbing::exporters::ExporterFactories;
use crate::otel::v2_callback::registry::V2Registry;

use self::hooks::RuntimeHooks;
use self::types::{LoggerRuntimeParts, V2Logger, V2LoggerConstructor};

pub mod error;
pub mod factory;
pub mod hooks;
pub mod legacy;
pub mod preset;
pub mod publish;
pub mod select;
pub mod types;

// litellm.integrations.otel.logger's module state as one injected value: the canonical callback, the V2
// catalog handles, the published provider and the fan-out registry the proxy shares with its auth adapter
pub struct LoggerRuntime {
    registered: Arc<V2Registry>,
    loggers: Mutex<Vec<Arc<V2Logger>>>,
    fan_out: Arc<Mutex<TenantFanOutRegistry>>,
    constructor: Arc<dyn V2LoggerConstructor>,
    base: Settings,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    trace: Arc<dyn RequestTraceSource>,
    factories: Arc<ExporterFactories>,
    scope: Arc<dyn RequestScopeSource>,
    assembler: Arc<dyn StreamOutputAssembler>,
    hooks: Option<Arc<dyn RuntimeHooks>>,
    configured_exclusions: Option<Vec<String>>,
    v2_enabled: bool,
    sdk_provider: SdkTracerProvider,
    generic: Mutex<Option<Arc<V2Logger>>>,
    published: Mutex<Option<SdkTracerProvider>>,
}

impl LoggerRuntime {
    #[must_use]
    pub fn new(parts: LoggerRuntimeParts) -> Self {
        Self {
            registered: parts.registered,
            loggers: Mutex::new(Vec::new()),
            fan_out: Arc::new(Mutex::new(TenantFanOutRegistry::default())),
            constructor: parts.constructor,
            base: parts.base,
            environment: parts.environment,
            trace: parts.trace,
            factories: parts.factories,
            scope: parts.scope,
            assembler: parts.assembler,
            hooks: parts.hooks,
            configured_exclusions: parts.configured_exclusions,
            v2_enabled: parts.v2_enabled,
            sdk_provider: parts.sdk_provider,
            generic: Mutex::new(None),
            published: Mutex::new(None),
        }
    }
}

// the runtime's one inherent impl holds the constructor; each area's logic is a free function in its module
fn loggers(runtime: &LoggerRuntime) -> MutexGuard<'_, Vec<Arc<V2Logger>>> {
    match runtime.loggers.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn generic(runtime: &LoggerRuntime) -> MutexGuard<'_, Option<Arc<V2Logger>>> {
    match runtime.generic.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn published(runtime: &LoggerRuntime) -> MutexGuard<'_, Option<SdkTracerProvider>> {
    match runtime.published.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[path = "logger_runtime/tests.rs"]
mod tests;
