use std::sync::Arc;

use crate::otel::config::Settings;

use super::error::LoggerRuntimeError;
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
    let request = LoggerBuildRequest {
        class,
        config: &config,
        callback_name,
        providers,
        assembler,
        settings,
    };
    let built = runtime.constructor.build(&request)?;
    let logger = Arc::new(V2Logger {
        callback: built.callback,
        provider: built.provider,
        config,
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
