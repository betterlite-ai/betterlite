mod adapter;
mod payload;
mod tests;

use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::braintrust_logging::logger::BraintrustLogger;
use crate::braintrust_logging::settings::{Settings, SettingsError};
use crate::callback_logger::CallbackLogger;
use crate::http_analytics_loggers::delivery::PooledDelivery;
use adapter::BraintrustAdapter;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings =
        Settings::resolve(None, None, Vec::new(), inputs.env.as_ref()).map_err(|error| {
            CatalogError::Init {
                message: init_message(&error),
            }
        })?;
    let logger = BraintrustLogger::new(
        settings,
        PooledDelivery {
            client: inputs.http.clone(),
        },
    );
    Ok(Arc::new(BraintrustAdapter {
        logger: Arc::new(logger),
        tracker: inputs.tracker.clone(),
    }))
}

// LiteLLM raises this text while building the logger, which set_callbacks reports and then skips it
fn init_message(error: &SettingsError) -> String {
    match error {
        SettingsError::MissingApiKey => {
            "Missing keys=['BRAINTRUST_API_KEY'] in environment.".to_owned()
        }
    }
}
