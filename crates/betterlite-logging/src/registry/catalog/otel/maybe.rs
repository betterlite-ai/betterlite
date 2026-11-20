use std::sync::Arc;

use betterlite_config::sections::otel_settings::FeatureFlag;

use super::super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::LoggerRuntime;
use crate::otel::logger_runtime::preset::maybe_construct_otel_v2;

// B3842's otel_v2 switch: litellm.integrations.otel.model.config.is_otel_v2_enabled()
pub fn v2_enabled(inputs: &CatalogInputs) -> Result<bool, CatalogError> {
    let mut flag = FeatureFlag::default();
    flag.enabled(inputs.env.as_ref())
        .map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })
}

// With the switch on but no runtime injected, the arm refuses its name rather than build the legacy logger.
pub fn require_runtime<'runtime>(
    runtime: Option<&'runtime LoggerRuntime>,
    name: &str,
) -> Result<&'runtime LoggerRuntime, CatalogError> {
    runtime.ok_or_else(|| CatalogError::Unavailable {
        name: name.to_owned(),
    })
}

// maybe_construct_otel_v2 already appends its logger, so an arm joins the same list once
pub(crate) fn push_once(list: &mut Vec<Arc<dyn CallbackLogger>>, logger: &Arc<dyn CallbackLogger>) {
    if !list.iter().any(|held| Arc::ptr_eq(held, logger)) {
        list.push(Arc::clone(logger));
    }
}

// litellm_logging._maybe_construct_otel_v2; Ok(None) means the arm builds its legacy logger instead
pub fn v2(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    name: &str,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Option<Arc<dyn CallbackLogger>>, CatalogError> {
    if !v2_enabled(inputs)? {
        return Ok(None);
    }
    let runtime = require_runtime(runtime, name)?;
    let built =
        maybe_construct_otel_v2(runtime, name, in_memory).map_err(|source| CatalogError::Init {
            message: source.to_string(),
        })?;
    Ok(built.map(|logger| Arc::clone(&logger.callback)))
}
