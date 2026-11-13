use std::sync::Arc;

use betterlite_config::sections::callback_settings::CallbackEntry;
use betterlite_config::sections::callback_settings::names::CallbackName;
use jiff::tz::TimeZone;

use super::{CatalogError, CatalogInputs};
use crate::callback_logger::CallbackLogger;
use crate::datadog::cost_management::datadog_cost_management::logger::{
    CostManagementDeps, DatadogCostManagementLogger,
};
use crate::datadog::cost_management::datadog_cost_management::settings::CostManagementInit;
use crate::datadog::cost_management::datadog_cost_management::transport::PooledTransport;

mod tests;

pub fn build(inputs: &CatalogInputs) -> Result<Arc<dyn CallbackLogger>, CatalogError> {
    let name = CallbackName::DatadogCostManagement.as_str();
    let params = inputs
        .callback_settings
        .get(name)
        .map(CallbackEntry::exposed_variables)
        .unwrap_or_default();
    let init = CostManagementInit::from_params(&params).map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    let logger = DatadogCostManagementLogger::new(
        &init,
        inputs.env.as_ref(),
        CostManagementDeps {
            transport: Arc::new(PooledTransport {
                client: inputs.http.clone(),
            }),
            zone: TimeZone::system(),
            tasks: Some(inputs.tracker.clone()),
            shutdown: inputs.shutdown.clone(),
        },
    )
    .map_err(|error| CatalogError::Init {
        message: error.to_string(),
    })?;
    Ok(logger)
}
