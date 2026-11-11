use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::datadog::metrics::datadog_metrics::{DatadogMetricsLogger, MetricsDeps};
use crate::datadog::metrics::settings::MetricsInit;
use crate::datadog_logging::clock::SystemClock;
use crate::datadog_logging::transport::PooledTransport;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let deps = MetricsDeps {
        transport: Arc::new(PooledTransport {
            client: inputs.http.clone(),
        }),
        clock: Arc::new(SystemClock),
        tasks: Some(inputs.tracker.clone()),
    };
    let logger = DatadogMetricsLogger::new(inputs.env.as_ref(), &MetricsInit::default(), deps)
        .map_err(|error| CatalogError::Init {
            message: error.to_string(),
        })?;
    Ok(logger)
}
