use std::sync::Arc;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;

pub fn build(_inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    Err(CatalogError::Unavailable {
        name: "weave_otel".to_owned(),
    })
}
