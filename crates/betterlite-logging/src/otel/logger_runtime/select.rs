use std::collections::BTreeSet;
use std::sync::Arc;

use tracing::warn;

use crate::callback_logger::CallbackLogger;
use crate::otel::config::Settings;
use crate::otel::vocabulary::resolvers::db_system;

use super::error::LoggerRuntimeError;
use super::factory::build_otel_v2_logger;
use super::types::{InjectedProviders, V2Logger};
use super::{LoggerRuntime, generic, loggers};

// litellm.integrations.otel.logger.select_global_otel_v2_logger
pub fn select_global_otel_v2_logger(
    runtime: &LoggerRuntime,
    in_memory: &[Arc<dyn CallbackLogger>],
) -> Result<Arc<V2Logger>, LoggerRuntimeError> {
    if let Some(registered) = runtime.registered.registered_v2_logger() {
        return Ok(Arc::new(V2Logger::from_callback(&registered)));
    }
    for callback in in_memory {
        if let Some(logger) = v2_logger_for(runtime, callback) {
            return Ok(logger);
        }
    }
    generic_logger(runtime)
}

// litellm.integrations.otel.logger._v2_configs: the selected config first, then the catalog's own
#[must_use]
pub fn v2_configs(
    runtime: &LoggerRuntime,
    in_memory: &[Arc<dyn CallbackLogger>],
    selected: &V2Logger,
) -> Vec<Settings> {
    let mut configs = vec![selected.config.clone()];
    for callback in in_memory {
        let Some(logger) = v2_logger_for(runtime, callback) else {
            continue;
        };
        if Arc::ptr_eq(&logger.callback, &selected.callback) {
            continue;
        }
        configs.push(logger.config.clone());
    }
    configs
}

// litellm.integrations.otel.logger._excluded_db_systems, with the explicit list injected as it was read
#[must_use]
pub fn excluded_db_systems(runtime: &LoggerRuntime, config: &Settings) -> BTreeSet<String> {
    match &runtime.configured_exclusions {
        None => config.excluded_services.clone(),
        Some(names) => normalized_exclusions(names),
    }
}

// a V2 logger is one this factory built, or B3853's registered callback; Rust has no isinstance
pub(super) fn v2_logger_for(
    runtime: &LoggerRuntime,
    callback: &Arc<dyn CallbackLogger>,
) -> Option<Arc<V2Logger>> {
    let held = loggers(runtime);
    if let Some(logger) = held
        .iter()
        .find(|logger| Arc::ptr_eq(&logger.callback, callback))
    {
        return Some(Arc::clone(logger));
    }
    drop(held);
    let registered = runtime.registered.registered_v2_logger()?;
    let erased: Arc<dyn CallbackLogger> = Arc::clone(&registered);
    Arc::ptr_eq(&erased, callback).then(|| Arc::new(V2Logger::from_callback(&registered)))
}

// the OTEL_* fallback logger, kept so a second selection cannot build a second orphan
fn generic_logger(runtime: &LoggerRuntime) -> Result<Arc<V2Logger>, LoggerRuntimeError> {
    let cached = generic(runtime).as_ref().cloned();
    if let Some(logger) = cached {
        return Ok(logger);
    }
    let logger = build_otel_v2_logger(
        runtime,
        runtime.base.clone(),
        None,
        InjectedProviders::default(),
    )?;
    *generic(runtime) = Some(Arc::clone(&logger));
    Ok(logger)
}

// B3842's normalization, applied to a callback_settings list that skipped the settings model
fn normalized_exclusions(names: &[String]) -> BTreeSet<String> {
    names
        .iter()
        .filter_map(|name| excluded_db_system(name))
        .collect()
}

fn excluded_db_system(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_lowercase();
    let system = if normalized == "postgresql" {
        Some("postgresql")
    } else {
        db_system(&normalized)
    };
    if system.is_none() {
        warn!(service = %name, "ignoring unknown OpenTelemetry excluded service");
    }
    system.map(str::to_owned)
}
