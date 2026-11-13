mod adapter;
mod payload;
mod tests;

use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use crate::http_analytics_loggers::promptlayer::{PromptLayerLogger, PromptLayerSettings};
use adapter::PromptLayerAdapter;

// the merged settings take any sized lookup; the catalog hands a trait object
struct Lookup<'env>(&'env (dyn EnvironmentLookup + Send + Sync));

impl EnvironmentLookup for Lookup<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        self.0.lookup(name)
    }
}

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings = PromptLayerSettings::from_environment(&Lookup(inputs.env.as_ref()));
    let logger = PromptLayerLogger {
        delivery: PooledDelivery {
            client: inputs.http.clone(),
        },
        settings,
    };
    Ok(Arc::new(PromptLayerAdapter {
        logger: Arc::new(logger),
        tracker: inputs.tracker.clone(),
    }))
}
