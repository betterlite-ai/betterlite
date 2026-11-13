mod adapter;
mod payload;
mod tests;

use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::finops::metering::clock::SystemClock;
use crate::finops::metering::openmeter::OpenMeterLogger;
use crate::finops::metering::openmeter::settings::{OpenMeterSettings, OpenMeterSettingsError};
use crate::finops::metering::transport::{DEFAULT_REQUEST_TIMEOUT, HttpMeteringSender};
use adapter::OpenMeterAdapter;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let environment: &dyn EnvironmentLookup = inputs.env.as_ref();
    let settings =
        OpenMeterSettings::from_environment(environment).map_err(|error| CatalogError::Init {
            message: init_message(error),
        })?;
    let sender = HttpMeteringSender::new(&inputs.http.policy, request_timeout(inputs)?).map_err(
        |error| CatalogError::Init {
            message: error.to_string(),
        },
    )?;
    Ok(Arc::new(OpenMeterAdapter {
        logger: Arc::new(OpenMeterLogger {
            settings,
            sender,
            clock: SystemClock,
        }),
        tracker: inputs.tracker.clone(),
    }))
}

fn request_timeout(inputs: &CatalogInputs) -> Result<Duration, CatalogError> {
    let Some(seconds) = inputs.config.litellm_settings.request_timeout else {
        return Ok(DEFAULT_REQUEST_TIMEOUT);
    };
    Duration::try_from_secs_f64(seconds).map_err(|error| CatalogError::Init {
        message: format!("litellm_settings.request_timeout is not a valid timeout: {error}"),
    })
}

// LiteLLM raises this text while building the logger, which set_callbacks reports and then skips it
fn init_message(error: OpenMeterSettingsError) -> String {
    match error {
        OpenMeterSettingsError::MissingKeys { missing_keys } => {
            let names: Vec<String> = missing_keys
                .iter()
                .map(|name| format!("'{name}'"))
                .collect();
            format!("Missing keys=[{}] in environment.", names.join(", "))
        }
        OpenMeterSettingsError::Environment(source) => source.to_string(),
    }
}
