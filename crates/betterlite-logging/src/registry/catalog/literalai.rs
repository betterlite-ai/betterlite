mod adapter;
mod payload;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::literal_ai::literal_ai_logging::LiteralAILogger;
use crate::literal_ai::literal_ai_logging::ids::SystemIds;
use crate::literal_ai::literal_ai_logging::settings::Overrides;
use adapter::LiteralAiAdapter;

const SETTING_VARIABLES: [&str; 5] = [
    "DEFAULT_BATCH_SIZE",
    "DEFAULT_FLUSH_INTERVAL_SECONDS",
    "LITERAL_API_KEY",
    "LITERAL_API_URL",
    "LITERAL_BATCH_SIZE",
];

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
    let environment = environment(inputs.env.as_ref());
    let logger = LiteralAILogger::from_environment(
        &environment,
        Overrides::default(),
        PooledDelivery {
            client: inputs.http.clone(),
        },
        SystemIds,
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(Arc::new(LiteralAiAdapter {
        logger: Arc::new(logger),
        tracker: inputs.tracker.clone(),
    }))
}
