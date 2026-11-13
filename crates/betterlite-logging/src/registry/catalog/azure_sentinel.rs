use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::azure_sentinel_logging::logger::{AzureSentinelLogger, SentinelDeps};
use crate::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};
use crate::callback_logger::CallbackLogger;
use crate::datadog_logging::clock::SystemClock;
use crate::datadog_logging::transport::PooledTransport;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings =
        SentinelSettings::resolve(&SentinelInit::default(), &*inputs.env).map_err(|error| {
            CatalogError::Init {
                message: error.to_string(),
            }
        })?;
    Ok(AzureSentinelLogger::new(
        settings,
        SentinelDeps {
            transport: Arc::new(PooledTransport {
                client: inputs.http.clone(),
            }),
            clock: Arc::new(SystemClock),
            stopped: inputs.shutdown.clone(),
            tasks: Some(inputs.tracker.clone()),
        },
    ))
}
