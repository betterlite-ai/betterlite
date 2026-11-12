mod adapter;
mod payload;
mod tests;

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::{Delivery, PooledDelivery};
use crate::opik::delivery::logger::{LoggerDependencies, OpikLogger};
use crate::opik::delivery::native::NoNativeClient;
use crate::opik::delivery::settings::LoggerOptions;
use adapter::OpikAdapter;

const SETTING_VARIABLES: [&str; 6] = [
    "DEFAULT_BATCH_SIZE",
    "DEFAULT_FLUSH_INTERVAL_SECONDS",
    "OPIK_API_KEY",
    "OPIK_PROJECT_NAME",
    "OPIK_URL_OVERRIDE",
    "OPIK_WORKSPACE",
];

const CONFIG_FILE_NAME: &str = ".opik.config";

// the merged settings read a map and the catalog hands them a lookup, so the names they read are copied over
pub(super) fn environment(
    lookup: &(dyn EnvironmentLookup + Send + Sync),
) -> BTreeMap<String, String> {
    SETTING_VARIABLES
        .iter()
        .filter_map(|name| lookup.lookup(name).map(|value| ((*name).to_owned(), value)))
        .collect()
}

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let logger = build_logger(
        inputs,
        PooledDelivery {
            client: inputs.http.clone(),
        },
    )?;
    Ok(Arc::new(OpikAdapter {
        logger,
        tracker: inputs.tracker.clone(),
    }))
}

// the merged flush loop runs on the catalog's tracker, and shutdown closes it so the loop flushes once more and exits
fn build_logger<D>(inputs: &CatalogInputs, delivery: D) -> Result<Arc<OpikLogger<D>>, CatalogError>
where
    D: Delivery + 'static,
{
    let environment = environment(inputs.env.as_ref());
    // get_opik_config_variable reads ~/.opik.config when neither a constructor value nor an environment variable answers
    let config_file = inputs
        .env
        .system_home()
        .and_then(|home| fs::read_to_string(home.join(CONFIG_FILE_NAME)).ok());
    let logger = OpikLogger::build(
        &LoggerOptions::default(),
        &environment,
        config_file.as_deref(),
        LoggerDependencies {
            delivery,
            native: &NoNativeClient,
            tasks: Some(&inputs.tracker),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let shutdown = inputs.shutdown.clone();
    let tracker = inputs.tracker.clone();
    let _watcher = inputs.tracker.spawn(async move {
        shutdown.cancelled().await;
        tracker.close();
    });
    Ok(logger)
}
