use std::sync::Arc;

use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::otel::config::Settings;
use crate::otel::v2_callback::error::CallbackError;
use crate::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_fan_out_tracer_provider,
};

use super::error::LoggerRuntimeError;
use super::select::excluded_db_systems;
use super::types::{
    BuiltLogger, ConstructorSettings, InjectedProviders, LoggerBuildRequest, LoggerClass, V2Logger,
};
use super::{LoggerRuntime, locked};

// litellm.integrations.otel.logger.build_otel_v2_logger: class choice, then the constructor port
pub fn build_otel_v2_logger(
    runtime: &LoggerRuntime,
    config: Settings,
    callback_name: Option<&str>,
    providers: InjectedProviders<'_>,
    settings: ConstructorSettings,
) -> Result<Arc<V2Logger>, LoggerRuntimeError> {
    let class = logger_class(&config);
    let assembler = matches!(class, LoggerClass::LangfuseContent).then_some(&runtime.assembler);
    let built_provider = providers
        .tracer_provider
        .is_none()
        .then(|| fan_out_provider_for(runtime, &config))
        .transpose()?;
    let request = LoggerBuildRequest {
        class,
        config: &config,
        callback_name,
        providers: InjectedProviders {
            tracer_provider: providers.tracer_provider.or(built_provider.as_ref()),
            ..providers
        },
        assembler,
        settings,
        factories: Arc::clone(&runtime.factories),
        scope_source: Arc::clone(&runtime.scope),
        trace: Arc::clone(&runtime.trace),
        environment: Arc::clone(&runtime.environment),
    };
    let built = runtime.constructor.build(&request)?;
    Ok(finish(runtime, built, config))
}

// logger.py:871: the OTEL_* fallback builds plain OpenTelemetryV2, so a config naming langfuse still gets that class
pub(super) fn build_generic_otel_v2_logger(
    runtime: &LoggerRuntime,
    config: Settings,
) -> Result<Arc<V2Logger>, LoggerRuntimeError> {
    let provider = fan_out_provider_for(runtime, &config)?;
    let request = LoggerBuildRequest {
        class: LoggerClass::Generic,
        config: &config,
        callback_name: None,
        providers: InjectedProviders {
            tracer_provider: Some(&provider),
            logger_provider: None,
            meter_provider: None,
        },
        assembler: None,
        settings: ConstructorSettings::default(),
        factories: Arc::clone(&runtime.factories),
        scope_source: Arc::clone(&runtime.scope),
        trace: Arc::clone(&runtime.trace),
        environment: Arc::clone(&runtime.environment),
    };
    let built = runtime.constructor.build(&request)?;
    Ok(finish(runtime, built, config))
}

// the provider a runtime-built logger publishes carries the tenant fan-out processor after its exporters
fn fan_out_provider_for(
    runtime: &LoggerRuntime,
    config: &Settings,
) -> Result<SdkTracerProvider, LoggerRuntimeError> {
    build_fan_out_tracer_provider(
        config,
        Arc::clone(&runtime.factories),
        &runtime.scope,
        AssemblyOptions {
            exporter: None,
            baggage_processor: None,
            use_simple_processor: None,
            tenant_overrides: true,
        },
        excluded_db_systems(runtime, config),
    )
    .map_err(|source| LoggerRuntimeError::Constructor(CallbackError::Exporter(source)))
}

fn finish(runtime: &LoggerRuntime, built: BuiltLogger, config: Settings) -> Arc<V2Logger> {
    let logger = Arc::new(V2Logger {
        callback: built.callback,
        hooks: built.hooks,
        provider: built.provider,
        config,
        recorder_providers: built.recorder_providers,
    });
    locked(&runtime.loggers).push(Arc::clone(&logger));
    logger
}

// litellm.integrations.otel.logger._logger_class: the exact mapper name "langfuse", never langfuse_otel
#[must_use]
pub fn logger_class(config: &Settings) -> LoggerClass {
    if !config.mapper_names.iter().any(|name| name == "langfuse") {
        return LoggerClass::Generic;
    }
    if config.capture_span_content() {
        return LoggerClass::LangfuseContent;
    }
    LoggerClass::Langfuse
}
