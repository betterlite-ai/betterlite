use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::posthog_logging::delivery::PooledTransport;
use crate::posthog_logging::event::uuid_id;
use crate::posthog_logging::logger::{PostHogDeps, PostHogLogger};
use crate::posthog_logging::settings::resolve;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let settings = resolve(inputs.env.as_ref()).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let logger = PostHogLogger::new(
        settings,
        PostHogDeps {
            transport: Arc::new(PooledTransport {
                client: inputs.http.clone(),
            }),
            ids: uuid_id,
            tasks: inputs.tracker.clone(),
            shutdown: Some(inputs.shutdown.clone()),
        },
    );
    Ok(logger)
}
