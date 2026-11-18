use std::sync::{Arc, Mutex, MutexGuard};

use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::callback_logger::CallbackLogger;
use crate::otel::config::Settings;
use crate::otel::content_projections::StreamOutputAssembler;
use crate::otel::fan_out::attach::TenantFanOutRegistry;
use crate::otel::v2_callback::callback::V2Callback;

use super::error::LoggerRuntimeError;

// litellm.integrations.otel.logger._logger_class's three callback forms
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggerClass {
    Generic,
    Langfuse,
    LangfuseContent,
}

// litellm.integrations.otel.logger.build_otel_v2_logger's tracer/logger/meter arguments
#[derive(Clone, Copy, Default)]
pub struct InjectedProviders<'a> {
    pub tracer_provider: Option<&'a SdkTracerProvider>,
    pub logger_provider: Option<&'a SdkLoggerProvider>,
    pub meter_provider: Option<&'a SdkMeterProvider>,
}

// CustomLogger.__init__'s turn_off_message_logging=False and deprecated message_logging=True
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstructorSettings {
    pub turn_off_message_logging: bool,
    pub message_logging: bool,
}

impl Default for ConstructorSettings {
    fn default() -> Self {
        Self {
            turn_off_message_logging: false,
            message_logging: true,
        }
    }
}

// what the injected constructor needs; it owns the V2Callback/Langfuse assembly the proxy wires elsewhere
pub struct LoggerBuildRequest<'a> {
    pub class: LoggerClass,
    pub config: &'a Settings,
    pub callback_name: Option<&'a str>,
    pub providers: InjectedProviders<'a>,
    pub assembler: Option<&'a Arc<dyn StreamOutputAssembler>>,
    pub settings: ConstructorSettings,
}

pub struct BuiltLogger {
    pub callback: Arc<dyn CallbackLogger>,
    pub provider: SdkTracerProvider,
}

// litellm.integrations.otel.logger._logger_class(...)(config=..., callback_name=..., **providers, **settings)
pub trait V2LoggerConstructor: Send + Sync {
    fn build(&self, request: &LoggerBuildRequest<'_>) -> Result<BuiltLogger, LoggerRuntimeError>;
}

// litellm.integrations.otel.logger.OpenTelemetryV2 as the selection and publication code sees it
pub struct V2Logger {
    pub callback: Arc<dyn CallbackLogger>,
    pub provider: SdkTracerProvider,
    pub config: Settings,
}

impl V2Logger {
    // the proxy's open_telemetry_logger slot as a view over B3853's registered callback
    #[must_use]
    pub fn from_callback(callback: &Arc<V2Callback>) -> Self {
        let erased: Arc<dyn CallbackLogger> = Arc::clone(callback);
        Self {
            config: callback.logger.config.clone(),
            provider: callback.tracer_provider.clone(),
            callback: erased,
        }
    }
}

// the provider fan_out_provider hands out together with the registry it already attached
pub struct FanOutProvider {
    pub provider: SdkTracerProvider,
    pub fan_out: Arc<Mutex<TenantFanOutRegistry>>,
}

impl FanOutProvider {
    #[must_use]
    pub fn registry(&self) -> MutexGuard<'_, TenantFanOutRegistry> {
        match self.fan_out.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
