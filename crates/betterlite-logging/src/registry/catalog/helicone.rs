mod adapter;
mod payload;
mod tests;

use std::sync::Arc;
use std::time::Duration;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::helicone::initialization::{HeliconeLogger, Settings};
use adapter::HeliconeAdapter;

// LiteLLM's shared logging client falls back to this total, so a dead intake cannot wedge a delivery forever
const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    Ok(Arc::new(HeliconeAdapter {
        logger: Arc::new(HeliconeLogger {
            settings: Settings::from_environment(inputs.env.as_ref()),
            client: inputs.http.clone(),
            timeout: REQUEST_TIMEOUT,
        }),
        tracker: inputs.tracker.clone(),
    }))
}
