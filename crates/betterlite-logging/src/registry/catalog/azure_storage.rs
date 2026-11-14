use std::sync::Arc;

use crate::azure_storage_logging::chain::UnwiredChainFactory;
use crate::azure_storage_logging::filename::RandomIds;
use crate::azure_storage_logging::logger::{AzureBlobStorageLogger, AzureStorageDeps};
use crate::azure_storage_logging::settings::AzureStorageSettings;
use crate::azure_storage_logging::transport::PooledTransport;
use crate::callback_logger::CallbackLogger;
use crate::datadog_logging::clock::SystemClock;

use super::{CatalogError, CatalogInputs};

mod tests;

// LiteLLM builds AzureBlobStorageLogger from the process environment when the callback is enabled
pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings =
        AzureStorageSettings::from_environment(inputs.env.as_ref()).map_err(|error| {
            CatalogError::Init {
                message: error.to_string(),
            }
        })?;
    let deps = AzureStorageDeps {
        transport: Arc::new(PooledTransport {
            client: inputs.http.clone(),
        }),
        clock: Arc::new(SystemClock),
        environment: Arc::clone(&inputs.env),
        ids: Arc::new(RandomIds),
        chain_factory: Arc::new(UnwiredChainFactory {
            environment: Arc::clone(&inputs.env),
        }),
        shutdown: inputs.shutdown.clone(),
        tasks: Some(inputs.tracker.clone()),
    };
    Ok(AzureBlobStorageLogger::new(settings, deps))
}
