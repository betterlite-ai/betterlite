use std::sync::Arc;

use super::{BaseLogger, CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;

pub fn build(_inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    Ok(Arc::new(BaseLogger {
        name: "pagerduty".to_owned(),
    }))
}
