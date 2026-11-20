use std::sync::Arc;

use super::otel::maybe;
use super::otel::shared::SharedOtel;
use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::newrelic_agent::backoff::MetricBackoff;
use crate::newrelic_agent::init::NewRelicInit;
use crate::newrelic_agent::logger::{NewRelicDeps, NewRelicLogger, PACKAGE_VERSION};
use crate::newrelic_extraction::ids::SystemUuids;
use crate::newrelic_metrics::clock::system_clock;
use crate::otel::logger_runtime::LoggerRuntime;

pub const CALLBACK_NAME: &str = "newrelic";

// litellm_logging's newrelic arm: V2 first, then the in-process NewRelicLogger, which reads its own variables
pub fn build(
    inputs: &CatalogInputs,
    runtime: Option<&LoggerRuntime>,
    _shared: &SharedOtel,
    in_memory: &mut Vec<Arc<dyn CallbackLogger>>,
) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    if let Some(logger) = maybe::v2(inputs, runtime, CALLBACK_NAME, in_memory)? {
        return Ok(logger);
    }
    let deps = NewRelicDeps {
        agent: None,
        environment: inputs.env.clone(),
        clock: system_clock(),
        ids: Arc::new(SystemUuids),
        backoff: Arc::new(MetricBackoff::default()),
        version: Some(PACKAGE_VERSION.to_owned()),
        failure_counters: Vec::new(),
    };
    Ok(NewRelicLogger::new(&NewRelicInit::default(), deps))
}

mod tests;
