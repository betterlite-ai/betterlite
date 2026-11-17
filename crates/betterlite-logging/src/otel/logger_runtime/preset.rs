use std::sync::Arc;

use tracing::warn;

use crate::callback_logger::CallbackLogger;
use crate::otel::config::{ExporterSpec, Settings};
use crate::otel::plumbing::context::destination_routing::destination_backends;
use crate::otel::presets::base::Preset;
use crate::otel::presets::preset_for;
use crate::otel::presets::utils::is_unconfigured_placeholder;

use super::LoggerRuntime;
use super::error::LoggerRuntimeError;
use super::factory::build_otel_v2_logger;
use super::select::v2_logger_for;
use super::types::{InjectedProviders, V2Logger};

// litellm.litellm_core_utils.litellm_logging._maybe_construct_otel_v2
pub fn maybe_construct_otel_v2(
    runtime: &LoggerRuntime,
    callback_name: &str,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Option<Arc<V2Logger>>, LoggerRuntimeError> {
    if !runtime.v2_enabled {
        return Ok(None);
    }
    let Some(preset) = preset_for(callback_name) else {
        return Ok(None);
    };
    let serves =
        destination_backends(&runtime.trace.trace_context().destinations).contains(callback_name);
    let has_v2 = in_memory
        .iter()
        .any(|callback| v2_logger_for(runtime, callback).is_some());
    let carried = serves && has_v2;
    if let Some(logger) = reusable_logger(runtime, in_memory, callback_name, serves) {
        return Ok(Some(logger));
    }
    let Some(config) = preset_config(runtime, preset, carried, callback_name) else {
        return Ok(None);
    };
    let logger = build_otel_v2_logger(
        runtime,
        config,
        Some(callback_name),
        InjectedProviders::default(),
    )?;
    in_memory.push(Arc::clone(&logger.callback));
    Ok(Some(logger))
}

// LiteLLM's same-name reuse loop: a destination for this callback or an exporter that goes somewhere
fn reusable_logger(
    runtime: &LoggerRuntime,
    in_memory: &[Arc<dyn CallbackLogger>],
    callback_name: &str,
    serves: bool,
) -> Option<Arc<V2Logger>> {
    for callback in in_memory {
        let Some(logger) = v2_logger_for(runtime, callback) else {
            continue;
        };
        if logger.callback.callback_name() == callback_name
            && (serves || !exports_nowhere(&logger.config))
        {
            return Some(logger);
        }
    }
    None
}

// only the preset sits inside the fallback: a constructor failure after this still surfaces
fn preset_config(
    runtime: &LoggerRuntime,
    preset: &dyn Preset,
    carried: bool,
    callback_name: &str,
) -> Option<Settings> {
    let Ok(built) = preset.build(runtime.environment.as_ref(), runtime.base.clone(), carried)
    else {
        return None;
    };
    let gated = is_credential_gated(&built);
    if gated && !carried && !has_operator_exporter(&built) {
        return None;
    }
    let config = if gated && carried {
        only_the_gated_exporter(&built)
    } else {
        built
    };
    if exports_nowhere(&config) {
        warn!(
            callback = callback_name,
            "OTel V2: no operator credentials; only key/team destinations will receive its traces"
        );
    }
    Some(config)
}

// litellm.litellm_core_utils.litellm_logging._is_gated
#[must_use]
pub fn is_gated(spec: &ExporterSpec) -> bool {
    if !spec.requires_headers {
        return false;
    }
    spec.headers.as_deref().is_none_or(str::is_empty)
}

// litellm.litellm_core_utils.litellm_logging._exports_nowhere: an empty exporter list exports nowhere
#[must_use]
pub fn exports_nowhere(config: &Settings) -> bool {
    for spec in &config.exporters {
        if !is_gated(spec) {
            return false;
        }
    }
    true
}

// litellm.litellm_core_utils.litellm_logging._is_credential_gated
#[must_use]
pub fn is_credential_gated(config: &Settings) -> bool {
    for spec in &config.exporters {
        if is_gated(spec) {
            return true;
        }
    }
    false
}

// litellm.litellm_core_utils.litellm_logging._has_operator_exporter
#[must_use]
pub fn has_operator_exporter(config: &Settings) -> bool {
    config
        .exporters
        .iter()
        .any(|spec| !is_gated(spec) && !is_unconfigured_placeholder(spec))
}

// litellm.litellm_core_utils.litellm_logging._only_the_gated_exporter
#[must_use]
pub fn only_the_gated_exporter(config: &Settings) -> Settings {
    let mut gated = config.clone();
    gated.exporters.retain(is_gated);
    gated
}
