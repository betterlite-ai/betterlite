use std::sync::Arc;

use betterlite_config::sections::callback_settings::CallbackEntry;
use betterlite_config::sections::callback_settings::names::CallbackName;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::legacy_s3::S3Logger;
use crate::legacy_s3::settings;
use crate::legacy_s3::transport;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let name = CallbackName::S3.as_str();
    let params = inputs
        .callback_settings
        .get(name)
        .map(CallbackEntry::exposed_variables)
        .unwrap_or_default();
    let resolved = settings::resolve(&params).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let upload_transport =
        transport::build(&resolved, Arc::clone(&inputs.env), inputs.http.clone()).map_err(
            |error| CatalogError::Init {
                message: error.to_string(),
            },
        )?;
    Ok(Arc::new(S3Logger::new(
        resolved,
        upload_transport,
        Arc::clone(&inputs.env),
        inputs.tracker.clone(),
    )))
}
