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
use self::types::{V2Logger, V2LoggerConstructor};

pub mod construction;
pub mod error;
pub mod factory;
pub mod hooks;
pub mod legacy;
pub mod preset;
pub mod publish;
pub mod select;
pub mod types;

// litellm.integrations.otel.logger's module state, shared with the auth adapter
pub struct LoggerRuntime {
    pub registered: Arc<V2Registry>,
    pub loggers: Mutex<Vec<Arc<V2Logger>>>,
    pub fan_out: Arc<Mutex<TenantFanOutRegistry>>,
    pub constructor: Arc<dyn V2LoggerConstructor>,
    pub base: Settings,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub trace: Arc<dyn RequestTraceSource>,
    pub factories: Arc<ExporterFactories>,
    pub scope: Arc<dyn RequestScopeSource>,
    pub assembler: Arc<dyn StreamOutputAssembler>,
    pub hooks: Option<Arc<dyn RuntimeHooks>>,
    pub configured_exclusions: Option<Vec<String>>,
    pub v2_enabled: bool,
    pub sdk_provider: SdkTracerProvider,
    pub generic: Mutex<Option<Arc<V2Logger>>>,
    pub published: Mutex<Option<SdkTracerProvider>>,
}

fn locked<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[path = "logger_runtime/tests.rs"]
pub(crate) mod tests;
