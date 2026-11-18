use std::sync::Arc;

use crate::otel::config::Settings;
use crate::otel::v2_callback::error::CallbackError;
use crate::otel_provider_assembly::tracer_provider_assembly::{
    AssemblyOptions, build_fan_out_tracer_provider,
};

use super::error::LoggerRuntimeError;
use super::select::excluded_db_systems;
use super::types::{
    ConstructorSettings, InjectedProviders, LoggerBuildRequest, LoggerClass, V2Logger,
};
use super::{LoggerRuntime, loggers};

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
    let built_provider = if providers.tracer_provider.is_some() {
        None
    } else {
        let provider = build_fan_out_tracer_provider(
            &config,
            Arc::clone(&runtime.factories),
            &runtime.scope,
            AssemblyOptions {
                exporter: None,
                baggage_processor: None,
                use_simple_processor: None,
                tenant_overrides: true,
            },
            excluded_db_systems(runtime, &config),
        )
        .map_err(|source| LoggerRuntimeError::Constructor {
            source: CallbackError::Exporter(source),
        })?;
        Some(provider)
    };
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
    let logger = Arc::new(V2Logger {
        callback: built.callback,
        provider: built.provider,
        config,
        recorder_providers: built.recorder_providers,
    });
    loggers(runtime).push(Arc::clone(&logger));
    Ok(logger)
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
